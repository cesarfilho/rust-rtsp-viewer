//! O daemon real publicando no MQTT (plano D3). Precisa de um broker (`RRV_TEST_MQTT=host:porta`; veja
//! `crates/rrv-core/tests/mqtt_broker.rs` para subir um); sem ele o teste avisa e passa.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rrv_core::testing::{LiveCamera, TempDir};
use rumqttc::{Client, Event, MqttOptions, Packet, QoS};

const DAEMON: &str = env!("CARGO_BIN_EXE_rrv-daemon");

#[test]
fn the_daemon_publishes_state_motion_and_goes_offline_on_sigterm() {
    let Some(addr) = std::env::var("RRV_TEST_MQTT").ok() else {
        eprintln!("sem RRV_TEST_MQTT: teste pulado");
        return;
    };
    let (host, port) = addr.split_once(':').unwrap();
    let prefix = format!("rrvd{}", std::process::id());

    // o assinante entra antes do daemon
    let mut opts = MqttOptions::new(
        format!("watch-{prefix}"),
        host,
        port.parse::<u16>().unwrap(),
    );
    opts.set_keep_alive(Duration::from_secs(5));
    let (client, mut conn) = Client::new(opts, 64);
    client
        .subscribe(format!("{prefix}/#"), QoS::AtLeastOnce)
        .unwrap();
    let (tx, rx) = mpsc::channel::<(String, String)>();
    std::thread::spawn(move || {
        for ev in conn.iter() {
            match ev {
                Ok(Event::Incoming(Packet::Publish(p))) => {
                    let _ = tx.send((p.topic, String::from_utf8_lossy(&p.payload).to_string()));
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });

    let tmp = TempDir::new("mqtt-daemon");
    let cam = LiveCamera::start(&tmp.0);
    let cfg = format!(
        "[recording]\ndir = {rec:?}\n\
         [motion]\nenabled = true\nsample_stride = 2\ncontour_area = 0.002\nthreshold = 20\n\
         [mqtt]\nenabled = true\nurl = \"mqtt://{addr}\"\nprefix = \"{prefix}\"\nclient_id = \"rrvd-{prefix}\"\n\
         [logs]\ndir = {logs:?}\n\
         [[cameras]]\nurl = {url:?}\nname = \"Portão\"\n",
        rec = tmp.path("rec"),
        logs = tmp.path("logs"),
        url = cam.url(),
    );
    let config = tmp.path("config.toml");
    std::fs::write(&config, cfg).unwrap();
    let mut child = Command::new(DAEMON)
        .arg(&config)
        .env("XDG_STATE_HOME", tmp.path("state"))
        .env("RRV_SOCKET", tmp.path("rrv.sock"))
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .expect("rrv-daemon");

    let mut online = false;
    let mut moving = false;
    let mut announced = false;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && !(online && moving && announced) {
        if let Ok((topic, body)) = rx.recv_timeout(Duration::from_millis(500)) {
            if topic == format!("{prefix}/portao/state") {
                online |= body.contains("\"online\":true");
                moving |= body.contains("\"motion\":true");
            }
            announced |= topic == format!("{prefix}/status") && body == "online";
        }
    }
    let _ = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status();
    let mut offline = false;
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end && !offline {
        if let Ok((topic, body)) = rx.recv_timeout(Duration::from_millis(500)) {
            offline = topic == format!("{prefix}/status") && body == "offline";
        }
    }
    let exited = child.wait().unwrap();
    assert!(announced, "o daemon não anunciou 'online'");
    assert!(online, "o estado da câmera nunca ficou online");
    assert!(moving, "o movimento da bola nunca chegou ao MQTT");
    assert!(offline, "o daemon não disse 'offline' ao sair");
    assert!(exited.success(), "{exited:?}");
}
