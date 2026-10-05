//! O socket do daemon: servidor e cliente reais sobre um socket Unix.
//! O motor fica nesta thread (como no daemon); o cliente roda em outra.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
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
use rrv_core::ipc::client::IpcClient;
use rrv_core::ipc::handler::Host;
use rrv_core::ipc::protocol::{PROTOCOL_VERSION, Request, Response, WireEvent};
use rrv_core::ipc::server::IpcServer;
use rrv_core::testing::TempDir;

fn engine(n: usize, tmp: &TempDir) -> Engine {
    let cams: Vec<CameraConfig> = (0..n)
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

/// Roda o servidor nesta thread por até `secs` ou até `client` terminar, e
/// devolve o que o cliente retornou.
fn with_server<T: Send + 'static>(
    tmp: &TempDir,
    sock: &Path,
    client: impl FnOnce() -> T + Send + 'static,
    mut each_tick: impl FnMut(&IpcServer),
) -> T {
    let mut engine = engine(2, tmp);
    let mut zones = ZonesFile::default();
    let server = IpcServer::bind(sock).unwrap();
    let handle = thread::spawn(client);
    let deadline = Instant::now() + Duration::from_secs(20);
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

#[test]
fn hello_and_status_round_trip_over_a_real_socket() {
    let tmp = TempDir::new("ipc-basic");
    let sock = tmp.path("rrv/rrv.sock");
    let s = sock.clone();
    let (server_name, response) = with_server(
        &tmp,
        &sock,
        move || {
            let mut c = IpcClient::connect(&s).unwrap();
            (c.server.clone(), c.request(&Request::Status).unwrap())
        },
        |_| {},
    );
    assert!(server_name.starts_with("rrv-daemon "), "{server_name}");
    let Response::Status { cameras } = response else {
        panic!("esperava Status, veio {response:?}");
    };
    assert_eq!(cameras.len(), 2);
    assert_eq!(cameras[0].name, "cam0");
}

#[test]
fn the_socket_is_private_and_removed_on_shutdown() {
    let tmp = TempDir::new("ipc-perms");
    let sock = tmp.path("rrv/rrv.sock");
    {
        let _server = IpcServer::bind(&sock).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&sock), 0o600, "só o dono fala com o daemon");
        assert_eq!(mode(sock.parent().unwrap()), 0o700);
    }
    assert!(!sock.exists(), "o socket some quando o servidor cai");
}

#[test]
fn a_stale_socket_file_is_replaced_but_a_live_daemon_is_not() {
    let tmp = TempDir::new("ipc-stale");
    let sock = tmp.path("rrv/rrv.sock");
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    // arquivo de um daemon que morreu sem limpar
    drop(std::os::unix::net::UnixListener::bind(&sock).unwrap());
    assert!(sock.exists());
    let first = IpcServer::bind(&sock).expect("deve substituir o socket velho");
    // com um daemon respondendo, o segundo não pode tomar o lugar
    let second = IpcServer::bind(&sock);
    assert!(
        second.is_err_and(|e| e.kind() == std::io::ErrorKind::AddrInUse),
        "dois daemons no mesmo socket"
    );
    drop(first);
}

fn raw(sock: &Path) -> (BufReader<UnixStream>, UnixStream) {
    let s = UnixStream::connect(sock).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    (BufReader::new(s.try_clone().unwrap()), s)
}

fn read(r: &mut BufReader<UnixStream>) -> String {
    let mut l = String::new();
    r.read_line(&mut l).unwrap();
    l
}

#[test]
fn a_wrong_protocol_version_is_refused_and_the_connection_closed() {
    let tmp = TempDir::new("ipc-version");
    let sock = tmp.path("rrv/rrv.sock");
    let s = sock.clone();
    let (reply, rest) = with_server(
        &tmp,
        &sock,
        move || {
            let (mut r, mut w) = raw(&s);
            w.write_all(b"{\"cmd\":\"hello\",\"protocol\":999}\n")
                .unwrap();
            let reply = read(&mut r);
            let mut rest = String::new();
            let n = r.read_line(&mut rest).unwrap();
            (reply, n)
        },
        |_| {},
    );
    assert!(reply.contains("não suportada"), "{reply}");
    assert_eq!(rest, 0, "o servidor fecha a conexão depois de recusar");
}

#[test]
fn the_first_message_must_be_hello() {
    let tmp = TempDir::new("ipc-first");
    let sock = tmp.path("rrv/rrv.sock");
    let s = sock.clone();
    let reply = with_server(
        &tmp,
        &sock,
        move || {
            let (mut r, mut w) = raw(&s);
            w.write_all(b"{\"cmd\":\"status\"}\n").unwrap();
            read(&mut r)
        },
        |_| {},
    );
    assert!(reply.contains("tem de ser Hello"), "{reply}");
}

#[test]
fn bad_json_gets_an_error_but_the_connection_survives() {
    let tmp = TempDir::new("ipc-badjson");
    let sock = tmp.path("rrv/rrv.sock");
    let s = sock.clone();
    let (err, status) = with_server(
        &tmp,
        &sock,
        move || {
            let (mut r, mut w) = raw(&s);
            let hello = format!("{{\"cmd\":\"hello\",\"protocol\":{PROTOCOL_VERSION}}}\n");
            w.write_all(hello.as_bytes()).unwrap();
            read(&mut r);
            w.write_all(b"isto nao e json\n").unwrap();
            let err = read(&mut r);
            w.write_all(b"{\"cmd\":\"status\"}\n").unwrap();
            (err, read(&mut r))
        },
        |_| {},
    );
    assert!(err.contains("mensagem inválida"), "{err}");
    assert!(status.contains("\"kind\":\"status\""), "{status}");
}

#[test]
fn an_oversized_line_closes_the_connection() {
    let tmp = TempDir::new("ipc-big");
    let sock = tmp.path("rrv/rrv.sock");
    let s = sock.clone();
    let closed = with_server(
        &tmp,
        &sock,
        move || {
            let (mut r, mut w) = raw(&s);
            let hello = format!("{{\"cmd\":\"hello\",\"protocol\":{PROTOCOL_VERSION}}}\n");
            w.write_all(hello.as_bytes()).unwrap();
            read(&mut r);
            // 2 MiB sem quebra de linha
            let _ = w.write_all(&vec![b'x'; 2 << 20]);
            let mut sink = Vec::new();
            r.read_to_end(&mut sink).is_ok()
        },
        |_| {},
    );
    assert!(closed, "o servidor deve encerrar a conexão");
}

#[test]
fn subscribers_receive_published_events_and_dead_ones_are_dropped() {
    let tmp = TempDir::new("ipc-events");
    let sock = tmp.path("rrv/rrv.sock");
    let s = sock.clone();
    let event = |n: u64| WireEvent {
        camera: 0,
        name: "cam0".into(),
        kind: EventType::Motion,
        detail: Some(format!("#{n}")),
        notification: None,
        unix_secs: n,
    };
    let published = std::cell::Cell::new(false);
    let received = with_server(
        &tmp,
        &sock,
        move || {
            // um assinante que vai embora e outro que fica
            let mut gone = IpcClient::connect(&s).unwrap();
            gone.subscribe().unwrap();
            drop(gone);
            let mut stay = IpcClient::connect(&s).unwrap();
            stay.subscribe().unwrap();
            let mut got = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(10);
            while got.len() < 2 && Instant::now() < deadline {
                if let Some(e) = stay.next_event(Duration::from_millis(200)).unwrap() {
                    got.push(e);
                }
            }
            got
        },
        |server| {
            // publica só depois de o cliente ter assinado (pedidos já foram atendidos)
            if !published.get() {
                thread::sleep(Duration::from_millis(400));
                server.publish(&[event(1), event(2)]);
                published.set(true);
            }
        },
    );
    assert_eq!(received.len(), 2, "{received:?}");
    assert_eq!(received[0].detail.as_deref(), Some("#1"));
    assert_eq!(received[1].unix_secs, 2);
}

#[test]
fn connecting_to_nothing_says_so_clearly() {
    let tmp = TempDir::new("ipc-none");
    let Err(err) = IpcClient::connect(&tmp.path("nao-existe.sock")) else {
        panic!("não devia conectar");
    };
    assert!(err.contains("ele está rodando?"), "{err}");
}
