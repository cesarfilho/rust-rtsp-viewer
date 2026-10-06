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
