//! O canal pela rede (TCP com token): servidor e cliente reais em 127.0.0.1. O motor fica nesta thread.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use rrv_core::config::{CameraConfig, LogsConfigFile};
use rrv_core::domain::motion::MotionConfig;
use rrv_core::domain::notify::NotifyConfig;
use rrv_core::domain::recording::RecordingConfig;
use rrv_core::domain::timeline::EventType;
use rrv_core::engine::{Engine, EngineSettings};
use rrv_core::infrastructure::zone_state::ZonesFile;
use rrv_core::ipc::auth;
use rrv_core::ipc::client::{ConnectError, IpcClient};
use rrv_core::ipc::handler::Host;
use rrv_core::ipc::protocol::{
    Request, Response, ServerMessage, WireEvent, decode_line, encode_line,
};
use rrv_core::ipc::server::IpcServer;
use rrv_core::testing::TempDir;

const TOKEN: &str = "token-de-teste-0123456789";

fn engine(tmp: &TempDir) -> Engine {
    let cams: Vec<CameraConfig> = (0..2)
        .map(|i| {
            toml::from_str(&format!(
                "url = \"rtsp://127.0.0.1:9/{i}\"\nname = \"cam{i}\""
            ))
            .unwrap()
        })
        .collect();
    Engine::new(EngineSettings {
        cameras: &cams,
        recording: &RecordingConfig::default(),
        motion: MotionConfig::default(),
        notify: NotifyConfig::default(),
        logs: &LogsConfigFile {
            dir: Some(tmp.path("logs")),
            ..LogsConfigFile::default()
        },
        pause_hidden: true,
        stagger: Duration::from_millis(100),
        zones: &ZonesFile::default(),
    })
}

/// Sobe o servidor (Unix + TCP) e roda o motor nesta thread até `client` terminar.
fn with_tcp_server<T: Send + 'static>(
    tmp: &TempDir,
    client: impl FnOnce(SocketAddr) -> T + Send + 'static,
    mut each_tick: impl FnMut(&IpcServer),
) -> T {
    let mut engine = engine(tmp);
    let mut zones = ZonesFile::default();
    let server = IpcServer::bind(&tmp.path("rrv/rrv.sock")).unwrap();
    let addr = server
        .listen_tcp("127.0.0.1:0".parse().unwrap(), TOKEN)
        .unwrap();
    let handle = thread::spawn(move || client(addr));
    let deadline = Instant::now() + Duration::from_secs(25);
    while !handle.is_finished() && Instant::now() < deadline {
        server.poll(&mut Host {
            engine: &mut engine,
            zones_file: &mut zones,
            persist: false,
            history: None,
            recordings: None,
        });
        each_tick(&server);
        thread::sleep(Duration::from_millis(10));
    }
    handle.join().expect("a thread do cliente entrou em pânico")
}

fn target(addr: SocketAddr) -> std::path::PathBuf {
    format!("tcp://{addr}").into()
}

#[test]
fn the_right_token_gets_hello_and_status_over_tcp() {
    let tmp = TempDir::new("tcp-ok");
    let (server, response) = with_tcp_server(
        &tmp,
        |addr| {
            let mut c = IpcClient::connect_target(&target(addr), Some(TOKEN)).unwrap();
            (c.server.clone(), c.request(&Request::Status).unwrap())
        },
        |_| {},
    );
    assert!(server.starts_with("rrv-daemon "), "{server}");
    let Response::Status { cameras } = response else {
        panic!("{response:?}");
    };
    assert_eq!(cameras.len(), 2);
}

#[test]
fn a_wrong_or_missing_token_never_reaches_the_engine() {
    let tmp = TempDir::new("tcp-bad");
    let (wrong, missing) = with_tcp_server(
        &tmp,
        |addr| {
            let wrong =
                IpcClient::connect_target(&target(addr), Some("outro-token-0123456789")).err();
            let missing = IpcClient::connect_target(&target(addr), None).err();
            (wrong, missing)
        },
        |_| {},
    );
    assert!(
        matches!(&wrong, Some(ConnectError::PermissionDenied(m)) if m.contains("recusou")),
        "{wrong:?}"
    );
    assert!(
        matches!(&missing, Some(ConnectError::PermissionDenied(m)) if m.contains("token")),
        "{missing:?}"
    );
}

#[test]
fn a_raw_client_that_skips_the_auth_is_dropped_before_any_request() {
    let tmp = TempDir::new("tcp-skip");
    let outcome = with_tcp_server(
        &tmp,
        |addr| {
            let mut s = TcpStream::connect(addr).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut r = BufReader::new(s.try_clone().unwrap());
            let mut line = String::new();
            r.read_line(&mut line).unwrap();
            let first: ServerMessage = decode_line(&line).unwrap();
            assert!(
                matches!(first, ServerMessage::Challenge { .. }),
                "{first:?}"
            );
            // pede o estado sem se autenticar
            s.write_all(encode_line(&Request::Status).as_bytes())
                .unwrap();
            line.clear();
            r.read_line(&mut line).unwrap();
            let reply: ServerMessage = decode_line(&line).unwrap();
            line.clear();
            let closed = r.read_line(&mut line).map_or(true, |n| n == 0);
            (reply, closed)
        },
        |_| {},
    );
    assert!(
        matches!(&outcome.0, ServerMessage::Response(Response::Error { message }) if message.contains("recusada")),
        "{:?}",
        outcome.0
    );
    assert!(outcome.1, "a conexão devia ser fechada");
}

#[test]
fn every_connection_gets_a_fresh_challenge_so_a_captured_answer_is_useless() {
    let tmp = TempDir::new("tcp-replay");
    let replay_refused = with_tcp_server(
        &tmp,
        |addr| {
            let read_challenge = || -> (TcpStream, BufReader<TcpStream>, String) {
                let s = TcpStream::connect(addr).unwrap();
                s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                let ServerMessage::Challenge { nonce } = decode_line(&line).unwrap() else {
                    panic!("sem desafio");
                };
                (s, r, nonce)
            };
            // conexão 1: responde certo e anota a resposta
            let (mut s1, mut r1, nonce1) = read_challenge();
            let answer1 = auth::respond(TOKEN, &nonce1);
            s1.write_all(
                encode_line(&Request::Auth {
                    response: answer1.clone(),
                })
                .as_bytes(),
            )
            .unwrap();
            let mut line = String::new();
            r1.read_line(&mut line).unwrap();
            assert!(matches!(
                decode_line::<ServerMessage>(&line).unwrap(),
                ServerMessage::Response(Response::Ok)
            ));
            // conexão 2: o desafio é outro; a resposta gravada da primeira não vale
            let (mut s2, mut r2, nonce2) = read_challenge();
            assert_ne!(nonce1, nonce2);
            s2.write_all(encode_line(&Request::Auth { response: answer1 }).as_bytes())
                .unwrap();
            line.clear();
            r2.read_line(&mut line).unwrap();
            matches!(
                decode_line::<ServerMessage>(&line).unwrap(),
                ServerMessage::Response(Response::Error { .. })
            )
        },
        |_| {},
    );
    assert!(replay_refused);
}

#[test]
fn events_reach_a_subscriber_over_tcp() {
    let tmp = TempDir::new("tcp-events");
    let mut sent = false;
    let event = with_tcp_server(
        &tmp,
        |addr| {
            let mut c = IpcClient::connect_target(&target(addr), Some(TOKEN)).unwrap();
            c.subscribe().unwrap();
            c.next_event(Duration::from_secs(10)).unwrap()
        },
        |server| {
            if !sent {
                sent = true;
                thread::sleep(Duration::from_millis(400));
            }
            server.publish(&[WireEvent {
                camera: 1,
                name: "cam1".into(),
                kind: EventType::Motion,
                detail: Some("teste".into()),
                notification: None,
                unix_secs: 1,
            }]);
        },
    );
    let event = event.expect("o evento devia chegar");
    assert_eq!((event.camera, event.kind), (1, EventType::Motion));
}

#[test]
fn a_server_that_is_not_a_daemon_is_reported_as_such_and_a_dead_port_as_not_running() {
    let tmp = TempDir::new("tcp-notdaemon");
    // um servidor TCP qualquer que responde lixo
    let junk = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let junk_addr = junk.local_addr().unwrap();
    thread::spawn(move || {
        if let Ok((mut s, _)) = junk.accept() {
            let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n");
        }
    });
    let _ = &tmp;
    let err = IpcClient::connect_target(&target(junk_addr), Some(TOKEN)).err();
    assert!(matches!(err, Some(ConnectError::Other(_))), "{err:?}");
    // nada escutando
    let dead = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap()
    };
    let err = IpcClient::connect_target(&target(dead), Some(TOKEN)).err();
    assert!(matches!(err, Some(ConnectError::NotRunning(_))), "{err:?}");
}

#[test]
fn a_weak_token_is_refused_when_listening() {
    let tmp = TempDir::new("tcp-weak");
    let server = IpcServer::bind(&tmp.path("rrv/rrv.sock")).unwrap();
    let err = server
        .listen_tcp("127.0.0.1:0".parse().unwrap(), "curto")
        .unwrap_err();
    assert!(err.to_string().contains("pelo menos"), "{err}");
    let _ = Path::new("");
}
