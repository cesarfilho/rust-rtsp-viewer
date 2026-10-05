//! O processo do daemon: sobe contra uma câmera HLS ao vivo simulada, grava por
//! movimento e, ao receber SIGTERM (o que `docker stop` envia), sai com código 0
//! deixando um arquivo de gravação completo e tocável.
//!
//! Antes do daemon, um SIGTERM durante a gravação deixava o arquivo com 0 bytes.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rrv_core::testing::{LiveCamera, TempDir, is_playable, mkv_files};

const DAEMON: &str = env!("CARGO_BIN_EXE_rrv-daemon");

fn write_config(tmp: &TempDir, url: &str) -> std::path::PathBuf {
    let cfg = format!(
        "[recording]\ndir = {rec:?}\non_motion = true\nmotion_post_roll_secs = 3600\n\
         [motion]\nsample_stride = 2\ncontour_area = 0.002\nthreshold = 20\n\
         [logs]\ndir = {logs:?}\n\
         [[cameras]]\nurl = {url:?}\nname = \"cam\"\n",
        rec = tmp.path("rec"),
        logs = tmp.path("logs"),
    );
    let path = tmp.path("config.toml");
    std::fs::write(&path, cfg).unwrap();
    path
}

fn wait_for_line(lines: &Arc<Mutex<Vec<String>>>, needle: &str, secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if lines.lock().unwrap().iter().any(|l| l.contains(needle)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn sigterm_during_a_recording_exits_cleanly_with_a_playable_file() {
    let tmp = TempDir::new("daemon");
    let cam = LiveCamera::start(&tmp.0); // a bola nunca para: a gravação não termina sozinha
    let config = write_config(&tmp, &cam.url());

    let mut child = Command::new(DAEMON)
        .arg(&config)
        .env("XDG_STATE_HOME", tmp.path("state"))
        .env("RUST_LOG", "info")
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("não consegui iniciar o rrv-daemon");
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = lines.clone();
    let stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            sink.lock().unwrap().push(line);
        }
    });

    let started = wait_for_line(&lines, "Recording started", 60);
    // deixa gravar um pouco
    std::thread::sleep(Duration::from_secs(4));
    let sent = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break Some(s);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = reader.join();
    let log = lines.lock().unwrap().join("\n");

    assert!(
        started,
        "a gravação por movimento nunca começou. Log:\n{log}"
    );
    assert!(sent.success(), "não consegui enviar SIGTERM");
    let status =
        status.unwrap_or_else(|| panic!("o daemon não saiu em 20 s após SIGTERM. Log:\n{log}"));
    assert!(status.success(), "saída anormal: {status:?}. Log:\n{log}");
    assert!(
        log.contains("finalizando gravações"),
        "não passou pelo desligamento limpo. Log:\n{log}"
    );

    let files = mkv_files(&tmp.path("rec"));
    assert_eq!(files.len(), 1, "{files:?}");
    let size = std::fs::metadata(&files[0]).unwrap().len();
    assert!(
        size > 1024,
        "arquivo quase vazio ({size} bytes). Log:\n{log}"
    );
    assert!(is_playable(&files[0]), "{} não toca", files[0].display());
}

#[test]
fn check_validates_the_config_and_exits_without_running() {
    let tmp = TempDir::new("daemon-check");
    let config = write_config(&tmp, "rtsp://127.0.0.1:9/never");
    let ok = Command::new(DAEMON)
        .arg("--check")
        .arg(&config)
        .output()
        .unwrap();
    assert!(ok.status.success(), "{ok:?}");
    assert!(String::from_utf8_lossy(&ok.stdout).contains("1 câmera(s)"));

    let missing = Command::new(DAEMON)
        .arg("--check")
        .arg("/nonexistent/rrv.toml")
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("cannot read"));
}
