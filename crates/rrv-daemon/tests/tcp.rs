//! O daemon de verdade escutando na rede: `rrvctl` conecta por `tcp://` com o token certo, e é recusado com um errado.

use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rrv_core::ipc::client::IpcClient;
use rrv_core::ipc::protocol::Response;
use rrv_core::testing::TempDir;

const DAEMON: &str = env!("CARGO_BIN_EXE_rrv-daemon");
const RRVCTL: &str = env!("CARGO_BIN_EXE_rrvctl");
const TOKEN: &str = "token-do-teste-0123456789abcdef";

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(tmp: &TempDir, extra: &str) -> (std::process::Child, std::path::PathBuf) {
    let config = format!(
        "[recording]\ndir = {rec:?}\n[logs]\ndir = {logs:?}\n{extra}\n[[cameras]]\nurl = \"rtsp://127.0.0.1:9/x\"\nname = \"Portão\"\n",
        rec = tmp.path("rec"),
        logs = tmp.path("logs"),
    );
    let path = tmp.path("config.toml");
    std::fs::write(&path, config).unwrap();
    let socket = tmp.path("rrv.sock");
    let child = Command::new(DAEMON)
        .arg(&path)
        .env("XDG_STATE_HOME", tmp.path("state"))
        .env("RRV_SOCKET", &socket)
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .expect("rrv-daemon");
    (child, socket)
}

fn wait_for(path: &std::path::Path) {
    let end = Instant::now() + Duration::from_secs(20);
    while !path.exists() && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(path.exists(), "o daemon não abriu o socket");
}

fn stop(mut child: std::process::Child) {
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let _ = child.wait();
}

#[test]
fn rrvctl_talks_to_a_daemon_over_the_network_with_the_token_only() {
    let tmp = TempDir::new("tcp-daemon");
    let port = free_port();
    let (child, socket) = start(
        &tmp,
        &format!("[daemon]\nlisten = \"127.0.0.1:{port}\"\ntoken = \"{TOKEN}\"\n"),
    );
    wait_for(&socket);
    // a porta TCP abre logo depois do socket
    let addr = format!("tcp://127.0.0.1:{port}");
    let end = Instant::now() + Duration::from_secs(10);
    let mut client = loop {
        match IpcClient::connect_target(std::path::Path::new(&addr), Some(TOKEN)) {
            Ok(c) => break c,
            Err(e) if Instant::now() > end => panic!("não conectou: {e}"),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let Response::Status { cameras } = client
        .request(&rrv_core::ipc::protocol::Request::Status)
        .unwrap()
    else {
        panic!("sem Status");
    };
    assert_eq!(cameras[0].name, "Portão");

    // a CLI de verdade, com o token pelo ambiente
    let out = Command::new(RRVCTL)
        .args(["--socket", &addr, "status"])
        .env("RRV_TOKEN", TOKEN)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Portão"));

    // token errado: recusado, sem vazar nada
    let bad = Command::new(RRVCTL)
        .args(["--socket", &addr, "status"])
        .env("RRV_TOKEN", "outro-token-0123456789abcdef")
        .env("RRV_KEYRING", "off")
        .output()
        .unwrap();
    assert!(!bad.status.success());
    let err = String::from_utf8_lossy(&bad.stderr);
    assert!(err.contains("recusou"), "{err}");
    assert!(!err.contains(TOKEN));
    // sem token nenhum
    let none = Command::new(RRVCTL)
        .args(["--socket", &addr, "status"])
        .env_remove("RRV_TOKEN")
        .env("RRV_KEYRING", "off")
        .env("RRV_SECRETS_DIR", tmp.path("nada"))
        .output()
        .unwrap();
    assert!(!none.status.success());
    assert!(String::from_utf8_lossy(&none.stderr).contains("token"));
    stop(child);
}

#[test]
fn a_daemon_without_listen_opens_no_tcp_port() {
    let tmp = TempDir::new("tcp-off");
    let (child, socket) = start(&tmp, "");
    wait_for(&socket);
    // o socket Unix atende; não há porta de rede
    assert!(IpcClient::connect(&socket).is_ok());
    stop(child);
}

#[test]
fn a_daemon_with_listen_but_a_weak_token_refuses_to_start() {
    let tmp = TempDir::new("tcp-weak");
    let port = free_port();
    let (mut child, _socket) = start(
        &tmp,
        &format!("[daemon]\nlisten = \"127.0.0.1:{port}\"\ntoken = \"curto\"\n"),
    );
    let end = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break Some(s);
        }
        if Instant::now() > end {
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    if status.is_none() {
        let _ = child.kill();
        panic!("o daemon devia ter recusado o token fraco");
    }
    assert!(!status.unwrap().success());
}

#[test]
fn the_daemon_serves_its_recordings_by_signed_url_to_a_client_on_the_network() {
    use rrv_core::ipc::protocol::Request;
    let tmp = TempDir::new("tcp-files");
    std::fs::create_dir_all(tmp.path("rec/Portao")).unwrap();
    let data: Vec<u8> = (0..5000u32).map(|i| (i % 199) as u8).collect();
    std::fs::write(tmp.path("rec/Portao/seg 1.mkv"), &data).unwrap();
    let port = free_port();
    let (child, socket) = start(
        &tmp,
        &format!("[daemon]\nlisten = \"127.0.0.1:{port}\"\ntoken = \"{TOKEN}\"\n"),
    );
    wait_for(&socket);
    let addr = format!("tcp://127.0.0.1:{port}");
    let end = Instant::now() + Duration::from_secs(10);
    let mut client = loop {
        match IpcClient::connect_target(std::path::Path::new(&addr), Some(TOKEN)) {
            Ok(c) => break c,
            Err(e) if Instant::now() > end => panic!("não conectou: {e}"),
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let Response::FileUrl {
        port: files_port,
        path,
    } = client
        .request(&Request::FileUrl {
            file: "Portao/seg 1.mkv".into(),
        })
        .unwrap()
    else {
        panic!("sem FileUrl");
    };
    // por padrão, a porta dos vídeos é a do canal + 1
    assert_eq!(files_port, port + 1);
    let url = format!("http://127.0.0.1:{files_port}{path}");
    // o `curl` de verdade, pedindo um trecho
    let out = Command::new("curl")
        .args(["-s", "-H", "Range: bytes=100-199", &url])
        .output()
        .unwrap();
    assert_eq!(out.stdout, &data[100..200]);
    let whole = Command::new("curl").args(["-s", &url]).output().unwrap();
    assert_eq!(whole.stdout, data);
    // fora da pasta: o daemon nem assina
    let Response::Error { .. } = client
        .request(&Request::FileUrl {
            file: "../config.toml".into(),
        })
        .unwrap()
    else {
        panic!("devia recusar");
    };
    stop(child);
}

#[test]
fn the_token_can_come_from_a_secret_file_like_in_docker() {
    let tmp = TempDir::new("tcp-secret");
    std::fs::create_dir_all(tmp.path("secrets")).unwrap();
    std::fs::write(tmp.path("secrets/rrv_token"), format!("{TOKEN}\n")).unwrap(); // com \n final, como o `openssl > arquivo`
    let port = free_port();
    let config = format!(
        "[recording]\ndir = {rec:?}\n[logs]\ndir = {logs:?}\n[daemon]\nlisten = \"127.0.0.1:{port}\"\ntoken = \"${{rrv_token}}\"\n[[cameras]]\nurl = \"rtsp://127.0.0.1:9/x\"\nname = \"Portão\"\n",
        rec = tmp.path("rec"),
        logs = tmp.path("logs"),
    );
    let path = tmp.path("config.toml");
    std::fs::write(&path, config).unwrap();
    let socket = tmp.path("rrv.sock");
    let child = Command::new(DAEMON)
        .arg(&path)
        .env("XDG_STATE_HOME", tmp.path("state"))
        .env("RRV_SOCKET", &socket)
        .env("RRV_SECRETS_DIR", tmp.path("secrets"))
        .env("RRV_KEYRING", "off")
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .expect("rrv-daemon");
    wait_for(&socket);
    let addr = format!("tcp://127.0.0.1:{port}");
    let end = Instant::now() + Duration::from_secs(10);
    let ok = loop {
        match IpcClient::connect_target(std::path::Path::new(&addr), Some(TOKEN)) {
            Ok(c) => break Some(c),
            Err(_) if Instant::now() < end => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break None,
        }
    };
    stop(child);
    assert!(ok.is_some(), "o token do arquivo de segredo não serviu");
}
