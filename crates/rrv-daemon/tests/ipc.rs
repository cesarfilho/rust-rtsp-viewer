//! O canal do daemon real: janela e `rrvctl` falam com ele pelo socket Unix.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rrv_core::domain::timeline::EventType;
use rrv_core::domain::zones::{MotionZoneFile, PointFile};
use rrv_core::ipc::client::IpcClient;
use rrv_core::ipc::protocol::{CameraInfo, Request, Response};
use rrv_core::testing::{LiveCamera, TempDir, is_playable, mkv_files};

const DAEMON: &str = env!("CARGO_BIN_EXE_rrv-daemon");
const RRVCTL: &str = env!("CARGO_BIN_EXE_rrvctl");

struct Daemon {
    child: Child,
    socket: PathBuf,
    state: PathBuf,
}

impl Daemon {
    fn start(tmp: &TempDir, url: &str, on_motion: bool) -> Self {
        let cfg = format!(
            "[recording]\ndir = {rec:?}\non_motion = {on_motion}\nmotion_post_roll_secs = 3600\n\
             [motion]\nsample_stride = 2\ncontour_area = 0.002\nthreshold = 20\n\
             [logs]\ndir = {logs:?}\n\
             [[cameras]]\nurl = {url:?}\nname = \"Portão\"\n",
            rec = tmp.path("rec"),
            logs = tmp.path("logs"),
        );
        let config = tmp.path("config.toml");
        std::fs::write(&config, cfg).unwrap();
        let socket = tmp.path("rrv.sock");
        let state = tmp.path("state");
        let child = Command::new(DAEMON)
            .arg(&config)
            .env("XDG_STATE_HOME", &state)
            .env("RRV_SOCKET", &socket)
            .env("RUST_LOG", "info")
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .expect("não consegui iniciar o rrv-daemon");
        let d = Self {
            child,
            socket,
            state,
        };
        let deadline = Instant::now() + Duration::from_secs(20);
        while !d.socket.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(d.socket.exists(), "o daemon não abriu o socket");
        d
    }

    fn client(&self) -> IpcClient {
        IpcClient::connect(&self.socket).unwrap()
    }

    fn stop(mut self) -> std::process::ExitStatus {
        let _ = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(s) = self.child.try_wait().unwrap() {
                return s;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!("o daemon não saiu em 20 s após SIGTERM");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn status(c: &mut IpcClient) -> Vec<CameraInfo> {
    match c.request(&Request::Status).unwrap() {
        Response::Status { cameras } => cameras,
        other => panic!("esperava Status: {other:?}"),
    }
}

fn wait_until(secs: u64, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

fn triangle() -> MotionZoneFile {
    MotionZoneFile {
        name: "porta".into(),
        vertices: vec![
            PointFile { x: 0.1, y: 0.1 },
            PointFile { x: 0.9, y: 0.1 },
            PointFile { x: 0.5, y: 0.9 },
        ],
        enabled: Some(true),
    }
}

/// Status, gravação manual e zonas: o que a janela fará pelo canal.
#[test]
fn the_window_can_control_the_daemon_over_the_socket() {
    let tmp = TempDir::new("dipc-control");
    let cam = LiveCamera::start(&tmp.0);
    cam.set_moving(false); // cena parada: nada grava sozinho
    let daemon = Daemon::start(&tmp, &cam.url(), false);
    let mut c = daemon.client();
    assert!(c.server.starts_with("rrv-daemon "));

    // a câmera sobe e fica ao vivo
    assert!(
        wait_until(40, || status(&mut c)[0].status == "live"),
        "a câmera não ficou ao vivo: {:?}",
        status(&mut c)
    );
    assert_eq!(status(&mut c)[0].name, "Portão");

    // gravação manual pelo canal
    let Response::Recording { recording, .. } =
        c.request(&Request::ToggleRecording { camera: 0 }).unwrap()
    else {
        panic!("esperava Recording");
    };
    assert!(recording);
    assert!(status(&mut c)[0].recording);
    std::thread::sleep(Duration::from_secs(4));
    let Response::Recording { recording, .. } =
        c.request(&Request::ToggleRecording { camera: 0 }).unwrap()
    else {
        panic!("esperava Recording");
    };
    assert!(!recording);

    // zonas: guardadas pelo nome da câmera, nunca pela URL
    assert_eq!(
        c.request(&Request::SetZones {
            camera: 0,
            zones: vec![triangle()]
        })
        .unwrap(),
        Response::Ok
    );
    let Response::Zones { zones, .. } = c.request(&Request::GetZones { camera: 0 }).unwrap() else {
        panic!("esperava Zones");
    };
    assert_eq!(zones, vec![triangle()]);
    let saved = std::fs::read_to_string(daemon.state.join("rust-rtsp-viewer/zones.toml"))
        .expect("as zonas devem ser persistidas");
    assert!(saved.contains("Portão"), "{saved}");
    assert!(
        !saved.contains("127.0.0.1"),
        "a URL não pode vazar: {saved}"
    );

    // câmera inexistente: erro claro, conexão segue viva
    let Response::Error { message } = c.request(&Request::GetZones { camera: 9 }).unwrap() else {
        panic!("esperava erro");
    };
    assert!(message.contains("não existe"), "{message}");
    assert_eq!(status(&mut c).len(), 1);

    assert!(daemon.stop().success());
    let files = mkv_files(&tmp.path("rec"));
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(is_playable(&files[0]), "a gravação manual não toca");
}

/// Quem assina recebe os eventos do motor em tempo real.
#[test]
fn a_subscriber_sees_motion_and_recording_events() {
    let tmp = TempDir::new("dipc-events");
    let cam = LiveCamera::start(&tmp.0); // a bola se mexe
    let daemon = Daemon::start(&tmp, &cam.url(), true);
    let mut c = daemon.client();
    c.subscribe().unwrap();

    let mut kinds = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !kinds.contains(&EventType::RecordingStart) {
        if let Some(ev) = c.next_event(Duration::from_millis(500)).unwrap() {
            assert_eq!(ev.name, "Portão");
            kinds.push(ev.kind);
        }
    }
    assert!(kinds.contains(&EventType::Motion), "{kinds:?}");
    assert!(kinds.contains(&EventType::RecordingStart), "{kinds:?}");
    assert!(daemon.stop().success());
}

/// O `rrvctl`, a ferramenta de linha de comando, usa o mesmo canal.
#[test]
fn rrvctl_talks_to_the_running_daemon() {
    let tmp = TempDir::new("dipc-ctl");
    let cam = LiveCamera::start(&tmp.0);
    cam.set_moving(false);
    let daemon = Daemon::start(&tmp, &cam.url(), false);
    let ctl = |args: &[&str]| {
        Command::new(RRVCTL)
            .args(args)
            .env("RRV_SOCKET", &daemon.socket)
            .output()
            .unwrap()
    };
    assert!(wait_until(40, || {
        let out = ctl(&["status", "--json"]);
        String::from_utf8_lossy(&out.stdout).contains("\"live\"")
    }));
    let text = ctl(&["status"]);
    let out = String::from_utf8_lossy(&text.stdout);
    assert!(out.contains("Portão") && out.contains("live"), "{out}");

    // por nome (sem diferenciar maiúsculas) e por índice
    assert!(ctl(&["record", "portão"]).status.success());
    assert!(String::from_utf8_lossy(&ctl(&["status"]).stdout).contains("sim"));
    assert!(ctl(&["record", "0"]).status.success());

    let bad = ctl(&["record", "garagem"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("não encontrada"));

    assert!(daemon.stop().success());
}

/// Dois daemons não podem gravar as mesmas câmeras; o socket some ao parar.
#[test]
fn a_second_daemon_is_refused_and_the_socket_is_removed_on_stop() {
    let tmp = TempDir::new("dipc-dup");
    let daemon = Daemon::start(&tmp, "rtsp://127.0.0.1:9/never", false);
    let second = Command::new(DAEMON)
        .arg(tmp.path("config.toml"))
        .env("XDG_STATE_HOME", tmp.path("state2"))
        .env("RRV_SOCKET", &daemon.socket)
        .output()
        .unwrap();
    assert!(!second.status.success());
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("já há um daemon"),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let socket: &Path = &daemon.socket.clone();
    assert!(daemon.stop().success());
    assert!(!socket.exists(), "o socket deve sumir ao desligar");
}

#[test]
fn a_client_without_a_daemon_gets_a_clear_message() {
    let tmp = TempDir::new("dipc-none");
    let out = Command::new(RRVCTL)
        .arg("status")
        .env("RRV_SOCKET", tmp.path("nada.sock"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("ele está rodando?"));
}

// ---------------------------------------------------------------- webhook ---

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

/// Cabeçalhos e corpo de cada POST recebido.
type Posts = Arc<Mutex<Vec<(Vec<String>, String)>>>;

/// Um receptor HTTP mínimo: guarda cada POST (cabeçalhos e corpo) e responde 200.
fn http_sink() -> (u16, Posts) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let got = Arc::new(Mutex::new(Vec::new()));
    let sink = got.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let sink = sink.clone();
            std::thread::spawn(move || {
                let mut r = BufReader::new(stream.try_clone().unwrap());
                let mut headers = Vec::new();
                let mut len = 0usize;
                loop {
                    let mut l = String::new();
                    if r.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
                        break;
                    }
                    if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                    headers.push(l.trim_end().to_string());
                }
                let mut body = vec![0u8; len];
                let _ = r.read_exact(&mut body);
                sink.lock()
                    .unwrap()
                    .push((headers, String::from_utf8_lossy(&body).to_string()));
                let mut w = stream;
                let _ = w.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            });
        }
    });
    (port, got)
}

/// O aviso com a janela fechada: movimento vira um POST JSON no destino, e a URL
/// (que carrega o token) não aparece no log do daemon.
#[test]
fn the_daemon_posts_motion_to_the_webhook_without_leaking_the_url() {
    let tmp = TempDir::new("dipc-webhook");
    let cam = LiveCamera::start(&tmp.0); // a bola se mexe
    let (port, received) = http_sink();
    let cfg = format!(
        "[recording]\ndir = {rec:?}\n\
         [motion]\nsample_stride = 2\ncontour_area = 0.002\nthreshold = 20\n\
         [webhook]\nurl = \"http://127.0.0.1:{port}/hook/TOKEN-SECRETO\"\n\
         [logs]\ndir = {logs:?}\n\
         [[cameras]]\nurl = {url:?}\nname = \"Portão\"\n",
        rec = tmp.path("rec"),
        logs = tmp.path("logs"),
        url = cam.url(),
    );
    std::fs::write(tmp.path("config.toml"), cfg).unwrap();
    let mut child = Command::new(DAEMON)
        .arg(tmp.path("config.toml"))
        .env("XDG_STATE_HOME", tmp.path("state"))
        .env("RRV_SOCKET", tmp.path("rrv.sock"))
        .env("RUST_LOG", "info")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let log = Arc::new(Mutex::new(String::new()));
    let sink = log.clone();
    let stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        for l in BufReader::new(stderr).lines().map_while(Result::ok) {
            let mut g = sink.lock().unwrap();
            g.push_str(&l);
            g.push('\n');
        }
    });

    let arrived = wait_until(60, || !received.lock().unwrap().is_empty());
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let status = child.wait().unwrap();
    let _ = reader.join();
    let log = log.lock().unwrap().clone();

    assert!(arrived, "nenhum POST chegou. Log:\n{log}");
    assert!(status.success(), "{status:?}\n{log}");
    let (headers, body) = received.lock().unwrap()[0].clone();
    assert!(
        headers
            .iter()
            .any(|h| h.starts_with("POST /hook/TOKEN-SECRETO")),
        "{headers:?}"
    );
    assert!(
        headers.iter().any(|h| h
            .to_ascii_lowercase()
            .starts_with("content-type: application/json")),
        "{headers:?}"
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("o corpo não é JSON");
    assert_eq!(v["camera"], "Portão");
    assert_eq!(v["event"], "motion");
    assert_eq!(v["title"], "Movimento detectado");
    assert!(
        v["message"].as_str().is_some_and(|m| m.contains("Portão")),
        "{v}"
    );
    assert!(
        log.contains("webhook ligado: http://127.0.0.1"),
        "o daemon deve anunciar o destino. Log:\n{log}"
    );
    assert!(!log.contains("TOKEN-SECRETO"), "a URL vazou no log:\n{log}");
}

/// Um destino fora do ar não atrapalha o daemon: continua saudável e desliga
/// rápido no SIGTERM (a fila é descartada, não esperada).
#[test]
fn a_dead_webhook_never_blocks_the_daemon_or_its_shutdown() {
    let tmp = TempDir::new("dipc-webhook-dead");
    let cam = LiveCamera::start(&tmp.0);
    // uma porta em que ninguém escuta
    let dead = {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let cfg = format!(
        "[recording]\ndir = {rec:?}\n\
         [motion]\nsample_stride = 2\ncontour_area = 0.002\nthreshold = 20\n\
         [webhook]\nurl = \"http://127.0.0.1:{dead}/hook/OUTRO-SEGREDO\"\ntimeout_secs = 2\n\
         [logs]\ndir = {logs:?}\n\
         [[cameras]]\nurl = {url:?}\nname = \"Portão\"\n",
        rec = tmp.path("rec"),
        logs = tmp.path("logs"),
        url = cam.url(),
    );
    std::fs::write(tmp.path("config.toml"), cfg).unwrap();
    let beat = tmp.path("beat");
    let mut child = Command::new(DAEMON)
        .arg(tmp.path("config.toml"))
        .arg("--health-file")
        .arg(&beat)
        .env("XDG_STATE_HOME", tmp.path("state"))
        .env("RRV_SOCKET", tmp.path("rrv.sock"))
        .env("RUST_LOG", "info")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let log = Arc::new(Mutex::new(String::new()));
    let sink = log.clone();
    let stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        for l in BufReader::new(stderr).lines().map_while(Result::ok) {
            let mut g = sink.lock().unwrap();
            g.push_str(&l);
            g.push('\n');
        }
    });
    // espera o aviso falhar (a bola gera movimento) e confere que o daemon vive
    assert!(
        wait_until(60, || log.lock().unwrap().contains("falhou")),
        "o webhook morto nunca foi tentado. Log:\n{}",
        log.lock().unwrap()
    );
    let health = Command::new(DAEMON)
        .arg("--health")
        .arg("--health-file")
        .arg(&beat)
        .output()
        .unwrap();
    assert!(
        health.status.success(),
        "o daemon travou com o webhook morto"
    );

    let started = Instant::now();
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let status = child.wait().unwrap();
    let took = started.elapsed();
    let _ = reader.join();
    let log = log.lock().unwrap().clone();

    assert!(status.success(), "{status:?}\n{log}");
    assert!(
        took < Duration::from_secs(8),
        "o desligamento esperou a fila do webhook: {took:?}"
    );
    assert!(!log.contains("OUTRO-SEGREDO"), "a URL vazou no log:\n{log}");
    assert!(log.contains("webhook para http://127.0.0.1"), "{log}");
}
