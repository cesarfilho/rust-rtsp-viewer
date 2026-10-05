//! Segredos fora do `config.toml` (plano 2.5.8): a senha da câmera vem de uma
//! variável de ambiente ou de um arquivo (Docker secrets), a câmera sobe com
//! ela e a senha não aparece em lugar nenhum do log.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rrv_core::ipc::client::IpcClient;
use rrv_core::ipc::protocol::{Request, Response};
use rrv_core::testing::{LiveCamera, TempDir};

const DAEMON: &str = env!("CARGO_BIN_EXE_rrv-daemon");

/// A URL da câmera de teste com um marcador no lugar da senha.
fn url_with_placeholder(cam: &LiveCamera, secret_name: &str) -> String {
    cam.url()
        .replace("http://", &format!("http://cam:${{{secret_name}}}@"))
}

fn config(tmp: &TempDir, url: &str) -> std::path::PathBuf {
    let cfg = format!(
        "[recording]\ndir = {rec:?}\n[logs]\ndir = {logs:?}\n[[cameras]]\nurl = {url:?}\nname = \"Portão\"\n",
        rec = tmp.path("rec"),
        logs = tmp.path("logs"),
    );
    let path = tmp.path("config.toml");
    std::fs::write(&path, cfg).unwrap();
    path
}

/// Sobe o daemon com `env`, espera a câmera ficar ao vivo e devolve o log inteiro.
fn run_until_live(tmp: &TempDir, cfg: &std::path::Path, envs: &[(&str, &str)]) -> (bool, String) {
    let mut cmd = Command::new(DAEMON);
    cmd.arg(cfg)
        .env("XDG_STATE_HOME", tmp.path("state"))
        .env("RRV_SOCKET", tmp.path("rrv.sock"))
        .env("RUST_LOG", "debug")
        .stderr(Stdio::piped())
        .stdout(Stdio::null());
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
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

    let mut live = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !live {
        std::thread::sleep(Duration::from_millis(300));
        if let Ok(mut c) = IpcClient::connect(&tmp.path("rrv.sock"))
            && let Ok(Response::Status { cameras }) = c.request(&Request::Status)
        {
            live = cameras.first().is_some_and(|c| c.status == "live");
        }
    }
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let _ = child.wait();
    let _ = reader.join();
    let out = log.lock().unwrap().clone();
    (live, out)
}

#[test]
fn a_password_from_the_environment_reaches_the_camera_and_never_the_log() {
    let tmp = TempDir::new("sec-env");
    let cam = LiveCamera::start(&tmp.0);
    let cfg = config(&tmp, &url_with_placeholder(&cam, "RRV_PASS_ENV"));
    // com @, / e : a URL quebraria sem o percent-encoding automático
    let secret = "p@ss/w:rd-Unico7";
    let (live, log) = run_until_live(&tmp, &cfg, &[("RRV_PASS_ENV", secret)]);

    assert!(
        live,
        "a câmera não subiu com a senha do ambiente. Log:\n{log}"
    );
    assert!(!log.contains(secret), "a senha vazou no log:\n{log}");
    assert!(
        !log.contains("p%40ss"),
        "a senha (codificada) vazou no log:\n{log}"
    );
    assert!(
        !log.contains("Unico7"),
        "parte da senha vazou no log:\n{log}"
    );
}

#[test]
fn a_password_from_a_secrets_file_works_like_docker_secrets() {
    let tmp = TempDir::new("sec-file");
    let cam = LiveCamera::start(&tmp.0);
    let dir = tmp.path("secrets");
    std::fs::create_dir_all(&dir).unwrap();
    // com o `\n` final que `echo` e os editores deixam
    std::fs::write(dir.join("cam_portao_password"), "S3cr3t!Arquivo9\n").unwrap();
    let cfg = config(&tmp, &url_with_placeholder(&cam, "cam_portao_password"));
    let (live, log) = run_until_live(&tmp, &cfg, &[("RRV_SECRETS_DIR", dir.to_str().unwrap())]);

    assert!(
        live,
        "a câmera não subiu com o segredo do arquivo. Log:\n{log}"
    );
    assert!(!log.contains("S3cr3t"), "o segredo vazou no log:\n{log}");
    assert!(
        !log.contains("Arquivo9"),
        "parte do segredo vazou no log:\n{log}"
    );
}

#[test]
fn a_missing_secret_stops_the_daemon_and_names_the_secret_only() {
    let tmp = TempDir::new("sec-missing");
    let cfg = config(
        &tmp,
        "rtsp://admin:${RRV_PASS_QUE_NAO_EXISTE}@127.0.0.1:9/s",
    );
    let out = Command::new(DAEMON)
        .arg(&cfg)
        .env("XDG_STATE_HOME", tmp.path("state"))
        .env("RRV_SOCKET", tmp.path("rrv.sock"))
        .env("RRV_SECRETS_DIR", tmp.path("nada"))
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "não pode subir sem a senha da câmera"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("RRV_PASS_QUE_NAO_EXISTE"), "{err}");
    assert!(err.contains("cameras[0] (Portão).url"), "{err}");
}

#[test]
fn check_warns_about_missing_secrets_instead_of_failing() {
    let tmp = TempDir::new("sec-check");
    let cfg = config(
        &tmp,
        "rtsp://admin:${RRV_PASS_QUE_NAO_EXISTE}@127.0.0.1:9/s",
    );
    let out = Command::new(DAEMON)
        .arg("--check")
        .arg(&cfg)
        .env("RRV_SECRETS_DIR", tmp.path("nada"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "o --check valida o arquivo sem exigir os segredos: {out:?}"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("aviso") && err.contains("RRV_PASS_QUE_NAO_EXISTE"),
        "{err}"
    );
}
