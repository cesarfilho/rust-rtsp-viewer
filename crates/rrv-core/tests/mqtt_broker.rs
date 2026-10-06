//! O MQTT contra um broker de verdade (plano D3). Precisa de um broker acessível: sem
//! `RRV_TEST_MQTT=host:porta` o teste avisa e passa. Para rodar um:
//!
//! ```text
//! printf 'listener 1883\nallow_anonymous true\n' > /tmp/mosq.conf
//! docker run -d --rm --name rrv-mosq -p 127.0.0.1:18830:1883 -v /tmp/mosq.conf:/mosquitto/config/mosquitto.conf eclipse-mosquitto:2
//! RRV_TEST_MQTT=127.0.0.1:18830 cargo test -p rrv-core --test mqtt_broker -- --nocapture
//! ```

use std::sync::mpsc;
use std::time::{Duration, Instant};

use rrv_core::domain::timeline::EventType;
use rrv_core::ipc::protocol::WireEvent;
use rrv_core::mqtt::{CameraState, MqttFile, MqttPublisher};
use rumqttc::{Client, Event, MqttOptions, Packet, QoS};

type Msg = (String, String, bool);

/// Um assinante de `#` que entrega `(tópico, corpo, retido)` por um canal.
fn subscriber(addr: &str, id: &str) -> (mpsc::Receiver<Msg>, Client) {
    let (host, port) = addr.split_once(':').unwrap();
    let mut opts = MqttOptions::new(id, host, port.parse::<u16>().unwrap());
    opts.set_keep_alive(Duration::from_secs(5));
    let (client, mut conn) = Client::new(opts, 64);
    client.subscribe("#", QoS::AtLeastOnce).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for ev in conn.iter() {
            match ev {
                Ok(Event::Incoming(Packet::Publish(p))) => {
                    let body = String::from_utf8_lossy(&p.payload).to_string();
                    if tx.send((p.topic, body, p.retain)).is_err() {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
    (rx, client)
}

fn drain(rx: &mpsc::Receiver<Msg>, secs: u64) -> Vec<Msg> {
    let end = Instant::now() + Duration::from_secs(secs);
    let mut out = Vec::new();
    while let Some(left) = end.checked_duration_since(Instant::now()) {
        if let Ok(m) = rx.recv_timeout(left) {
            out.push(m);
        }
    }
    out
}

fn config(addr: &str, prefix: &str) -> rrv_core::mqtt::MqttConfig {
    toml::from_str::<MqttFile>(&format!(
        "enabled = true\nurl = \"mqtt://{addr}\"\nprefix = \"{prefix}\"\nclient_id = \"rrv-test-{prefix}\""
    ))
    .unwrap()
    .into_config()
    .unwrap()
}

#[test]
fn state_events_discovery_and_availability_reach_a_real_broker() {
    let Some(addr) = std::env::var("RRV_TEST_MQTT").ok() else {
        eprintln!("sem RRV_TEST_MQTT (veja o topo do arquivo): teste pulado");
        return;
    };
    let prefix = format!("rrvtest{}", std::process::id());
    let (rx, _watch) = subscriber(&addr, &format!("watch-{prefix}"));
    std::thread::sleep(Duration::from_millis(500));

    let names = vec!["Portão".to_string(), "Sala".to_string()];
    let p = MqttPublisher::spawn(config(&addr, &prefix), &names).unwrap();
    let seen = drain(&rx, 3);
    let topics: Vec<&str> = seen.iter().map(|m| m.0.as_str()).collect();
    assert!(
        seen.iter()
            .any(|m| m.0 == format!("{prefix}/status") && m.1 == "online"),
        "disponibilidade: {topics:?}"
    );
    let discoveries = seen
        .iter()
        .filter(|m| {
            m.0.starts_with("homeassistant/") && m.0.contains("/rrv_portao/")
                || m.0.contains("/rrv_sala/")
        })
        .count();
    assert!(
        discoveries >= 8,
        "descoberta das duas câmeras: {discoveries} em {topics:?}"
    );

    // estado: o mesmo duas vezes vira uma publicação só; uma mudança vira outra
    let mut s = CameraState {
        status: "live".into(),
        online: true,
        ..CameraState::default()
    };
    p.update(0, &s);
    p.update(0, &s);
    s.motion = true;
    s.objects = vec!["person".into()];
    p.update(0, &s);
    let ev = WireEvent {
        camera: 0,
        name: "Portão".into(),
        kind: EventType::Detection,
        detail: Some("person 90%".into()),
        notification: None,
        unix_secs: 1_700_000_000,
    };
    p.event(&ev);
    let seen = drain(&rx, 2);
    let states: Vec<&Msg> = seen
        .iter()
        .filter(|m| m.0 == format!("{prefix}/portao/state"))
        .collect();
    assert_eq!(states.len(), 2, "dedupe do estado: {seen:?}");
    assert!(states[0].1.contains("\"motion\":false"));
    assert!(states[1].1.contains("\"motion\":true") && states[1].1.contains("person"));
    let events: Vec<&Msg> = seen
        .iter()
        .filter(|m| m.0 == format!("{prefix}/portao/event"))
        .collect();
    assert_eq!(events.len(), 1);
    assert!(!events[0].2, "eventos não são retidos");
    assert!(events[0].1.contains("\"event\":\"detection\""));

    // quem chega depois recebe o estado retido e a descoberta
    let (rx2, _late) = subscriber(&addr, &format!("late-{prefix}"));
    let late = drain(&rx2, 2);
    let retained = |topic: String, needle: &str| {
        late.iter()
            .any(|m| m.0 == topic && m.1.contains(needle) && m.2)
    };
    assert!(
        retained(format!("{prefix}/portao/state"), "person"),
        "estado retido para quem chega depois: {late:?}"
    );
    assert!(retained(format!("{prefix}/status"), "online"), "{late:?}");
    assert!(
        retained(
            "homeassistant/binary_sensor/rrv_sala/motion/config".into(),
            "rrv_sala_motion"
        ),
        "descoberta retida: {late:?}"
    );

    // desligar diz offline
    p.shutdown();
    let end = drain(&rx, 2);
    assert!(
        end.iter()
            .any(|m| m.0 == format!("{prefix}/status") && m.1 == "offline"),
        "offline ao encerrar: {end:?}"
    );
}

#[test]
fn a_dead_broker_never_blocks_or_fails_the_publisher() {
    let p = MqttPublisher::spawn(config("127.0.0.1:9", "naoexiste"), &["A".to_string()]).unwrap();
    let t = Instant::now();
    for i in 0..500 {
        p.update(
            0,
            &CameraState {
                status: format!("s{i}"),
                ..CameraState::default()
            },
        );
    }
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    p.shutdown();
}
