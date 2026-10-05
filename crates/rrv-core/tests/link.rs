//! O `DaemonLink`: conexão da janela com o daemon (heartbeat, backoff,
//! classificação de falhas). Servidor real num lado, servidores falsos nos casos
//! de falha.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rrv_core::config::{CameraConfig, LogsConfigFile};
use rrv_core::domain::motion::MotionConfig;
use rrv_core::domain::notify::NotifyConfig;
use rrv_core::domain::recording::RecordingConfig;
use rrv_core::domain::timeline::EventType;
use rrv_core::engine::{Engine, EngineSettings};
use rrv_core::infrastructure::zone_state::ZonesFile;
use rrv_core::ipc::handler::Host;
use rrv_core::ipc::link::{DaemonLink, LinkEvent};
use rrv_core::ipc::protocol::{Request, Response, WireEvent};
use rrv_core::ipc::server::IpcServer;
use rrv_core::testing::TempDir;

/// Um daemon de mentira: motor sem câmeras rodando + servidor, numa thread.
struct FakeDaemon {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    publish: std::sync::mpsc::Sender<WireEvent>,
}

impl FakeDaemon {
    fn start(tmp: &TempDir, sock: &Path) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (publish, rx) = std::sync::mpsc::channel::<WireEvent>();
        let (flag, sock, logs) = (stop.clone(), sock.to_path_buf(), tmp.path("logs"));
        let thread = thread::spawn(move || {
            let cam: CameraConfig =
                toml::from_str("url = \"rtsp://127.0.0.1:9/x\"\nname = \"cam\"").unwrap();
            let mut engine = Engine::new(EngineSettings {
                cameras: &[cam],
                recording: &RecordingConfig::default(),
                motion: MotionConfig::default(),
                notify: NotifyConfig::default(),
                logs: &LogsConfigFile {
                    dir: Some(logs),
                    ..LogsConfigFile::default()
                },
                pause_hidden: true,
                stagger: Duration::from_millis(100),
                zones: &ZonesFile::default(),
            });
            let mut zones = ZonesFile::default();
            let server = IpcServer::bind(&sock).unwrap();
            while !flag.load(Ordering::Relaxed) {
                server.poll(&mut Host {
                    engine: &mut engine,
                    zones_file: &mut zones,
                    persist: false,
                    history: None,
                });
                let evs: Vec<_> = rx.try_iter().collect();
                server.publish(&evs);
                thread::sleep(Duration::from_millis(10));
            }
        });
        // espera o socket existir
        let deadline = Instant::now() + Duration::from_secs(5);
        while !PathBuf::from(&sock_path(tmp)).exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        Self {
            stop,
            thread: Some(thread),
            publish,
        }
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn sock_path(tmp: &TempDir) -> PathBuf {
    tmp.path("rrv/rrv.sock")
}

impl Drop for FakeDaemon {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Espera um evento que satisfaça `pick` por até `secs`.
fn wait_for<T>(
    link: &DaemonLink,
    secs: u64,
    mut pick: impl FnMut(&LinkEvent) -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if let Ok(e) = link.events.recv_timeout(Duration::from_millis(100))
            && let Some(v) = pick(&e)
        {
            return Some(v);
        }
    }
    None
}

#[test]
fn connects_reports_the_cameras_and_keeps_the_status_fresh() {
    let tmp = TempDir::new("link-ok");
    let sock = sock_path(&tmp);
    let _daemon = FakeDaemon::start(&tmp, &sock);
    let link = DaemonLink::spawn(sock);

    let (server, cameras) = wait_for(&link, 10, |e| match e {
        LinkEvent::Connected { server, cameras } => Some((server.clone(), cameras.clone())),
        _ => None,
    })
    .expect("nunca conectou");
    assert!(server.starts_with("rrv-daemon "));
    assert_eq!(cameras.len(), 1);
    assert_eq!(cameras[0].name, "cam");

    // o heartbeat chega sem ninguém pedir
    assert!(
        wait_for(&link, 5, |e| matches!(e, LinkEvent::Status(_))
            .then_some(()))
        .is_some(),
        "o status não é atualizado sozinho"
    );
}

#[test]
fn forwards_events_in_real_time() {
    let tmp = TempDir::new("link-events");
    let sock = sock_path(&tmp);
    let daemon = FakeDaemon::start(&tmp, &sock);
    let link = DaemonLink::spawn(sock);
    wait_for(&link, 10, |e| {
        matches!(e, LinkEvent::Connected { .. }).then_some(())
    })
    .expect("nunca conectou");

    daemon
        .publish
        .send(WireEvent {
            camera: 0,
            name: "cam".into(),
            kind: EventType::Motion,
            detail: Some("5%".into()),
            notification: None,
            unix_secs: 1,
        })
        .unwrap();
    let got = wait_for(&link, 5, |e| match e {
        LinkEvent::Event(ev) => Some(ev.clone()),
        _ => None,
    })
    .expect("o evento não chegou");
    assert_eq!(got.kind, EventType::Motion);
    assert_eq!(got.detail.as_deref(), Some("5%"));
}

#[test]
fn requests_get_a_reply_with_their_token() {
    let tmp = TempDir::new("link-reply");
    let sock = sock_path(&tmp);
    let _daemon = FakeDaemon::start(&tmp, &sock);
    let link = DaemonLink::spawn(sock);
    wait_for(&link, 10, |e| {
        matches!(e, LinkEvent::Connected { .. }).then_some(())
    })
    .expect("nunca conectou");

    link.request(42, Request::GetZones { camera: 0 });
    let result = wait_for(&link, 5, |e| match e {
        LinkEvent::Reply { token: 42, result } => Some(result.clone()),
        _ => None,
    })
    .expect("sem resposta");
    assert!(matches!(result, Ok(Response::Zones { .. })), "{result:?}");

    // erro do daemon volta como resposta, não como queda da conexão
    link.request(43, Request::GetZones { camera: 9 });
    let result = wait_for(&link, 5, |e| match e {
        LinkEvent::Reply { token: 43, result } => Some(result.clone()),
        _ => None,
    })
    .unwrap();
    assert!(matches!(result, Ok(Response::Error { .. })), "{result:?}");
}

#[test]
fn losing_the_daemon_is_reported_and_it_reconnects_when_it_returns() {
    let tmp = TempDir::new("link-lost");
    let sock = sock_path(&tmp);
    let mut daemon = FakeDaemon::start(&tmp, &sock);
    let link = DaemonLink::spawn(sock.clone());
    wait_for(&link, 10, |e| {
        matches!(e, LinkEvent::Connected { .. }).then_some(())
    })
    .expect("nunca conectou");

    daemon.stop(); // o daemon vai embora
    let reason = wait_for(&link, 10, |e| match e {
        LinkEvent::Lost { reason, .. } => Some(reason.clone()),
        _ => None,
    })
    .expect("não percebeu que o daemon caiu");
    assert!(!reason.is_empty());

    let _back = FakeDaemon::start(&tmp, &sock); // e volta
    assert!(
        wait_for(&link, 25, |e| matches!(e, LinkEvent::Connected { .. })
            .then_some(()))
        .is_some(),
        "não reconectou sozinha"
    );
}

/// Servidor falso que fala só o suficiente: responde ao Hello com `hello`.
fn fake_server(sock: &Path, hello: &'static str, answer_after_hello: usize) -> JoinHandle<()> {
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    let listener = UnixListener::bind(sock).unwrap();
    thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        let mut w = stream.try_clone().unwrap();
        let mut r = BufReader::new(stream);
        let mut line = String::new();
        r.read_line(&mut line).unwrap(); // Hello
        w.write_all(hello.as_bytes()).unwrap();
        // responde a `answer_after_hello` pedidos e depois fica mudo
        for i in 0..answer_after_hello {
            line.clear();
            if r.read_line(&mut line).unwrap_or(0) == 0 {
                return;
            }
            let reply = if line.contains("subscribe") {
                "{\"type\":\"response\",\"kind\":\"subscribed\"}\n"
            } else {
                "{\"type\":\"response\",\"kind\":\"status\",\"cameras\":[]}\n"
            };
            let _ = i;
            w.write_all(reply.as_bytes()).unwrap();
        }
        // mudo: mantém a conexão aberta sem responder
        thread::sleep(Duration::from_secs(30));
    })
}

#[test]
fn a_protocol_mismatch_is_incompatible_and_does_not_retry_by_itself() {
    let tmp = TempDir::new("link-incompat");
    let sock = sock_path(&tmp);
    let _server = fake_server(
        &sock,
        "{\"type\":\"response\",\"kind\":\"error\",\"message\":\"versão do protocolo 1 não suportada (o daemon fala 2)\"}\n",
        0,
    );
    let link = DaemonLink::spawn(sock);
    let message = wait_for(&link, 10, |e| match e {
        LinkEvent::Incompatible { message } => Some(message.clone()),
        _ => None,
    })
    .expect("não classificou como incompatível");
    assert!(message.contains("não suportada"), "{message}");
    // não há nova tentativa sozinha: nada de Connecting/Lost nos próximos segundos
    let again = wait_for(&link, 4, |e| {
        matches!(e, LinkEvent::Connecting | LinkEvent::Lost { .. }).then_some(())
    });
    assert!(again.is_none(), "tentou de novo sozinha");
}

#[test]
fn a_socket_without_permission_is_reported_as_such() {
    let tmp = TempDir::new("link-perm");
    let sock = sock_path(&tmp);
    std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
    let _listener = UnixListener::bind(&sock).unwrap();
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o000)).unwrap();
    // como root a permissão não se aplica
    if std::fs::File::open("/proc/self/mem").is_ok()
        && std::env::var("USER").as_deref() == Ok("root")
    {
        return;
    }
    let link = DaemonLink::spawn(sock);
    let got = wait_for(&link, 10, |e| match e {
        LinkEvent::NoPermission { message } => Some(message.clone()),
        _ => None,
    });
    assert!(got.is_some(), "não classificou como sem permissão");
}

/// O heartbeat: um daemon que aceita a conexão mas para de responder é dado como
/// perdido em até `HEARTBEAT_TIMEOUT`.
#[test]
fn a_silent_daemon_is_lost_within_the_heartbeat_timeout() {
    let tmp = TempDir::new("link-silent");
    let sock = sock_path(&tmp);
    let _server = fake_server(
        &sock,
        "{\"type\":\"response\",\"kind\":\"hello\",\"protocol\":1,\"server\":\"rrv-daemon fake\"}\n",
        2, // subscribe + primeiro status; depois, mudo
    );
    let link = DaemonLink::spawn(sock);
    wait_for(&link, 10, |e| {
        matches!(e, LinkEvent::Connected { .. }).then_some(())
    })
    .expect("nunca conectou");
    let started = Instant::now();
    let reason = wait_for(&link, 15, |e| match e {
        LinkEvent::Lost { reason, .. } => Some(reason.clone()),
        _ => None,
    })
    .expect("não percebeu que o daemon ficou mudo");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "demorou {:?} para perceber: {reason}",
        started.elapsed()
    );
}

#[test]
fn requests_without_a_connection_fail_clearly() {
    let tmp = TempDir::new("link-offline");
    let link = DaemonLink::spawn(tmp.path("nada.sock"));
    wait_for(&link, 5, |e| {
        matches!(e, LinkEvent::Lost { .. }).then_some(())
    })
    .expect("devia estar perdido");
    link.request(7, Request::Status);
    let result = wait_for(&link, 5, |e| match e {
        LinkEvent::Reply { token: 7, result } => Some(result.clone()),
        _ => None,
    })
    .expect("sem resposta");
    assert!(result.is_err(), "{result:?}");
}
