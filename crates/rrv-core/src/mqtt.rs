//! MQTT para o Home Assistant (plano D3): o daemon publica o estado de cada câmera e os eventos, e se
//! anuncia ao Home Assistant pela descoberta automática (MQTT Discovery).
//!
//! Dividido em duas metades. A de cima é **pura** — nomes de tópico, corpo do estado, mensagens de
//! descoberta, leitura da URL — e é testada sem broker. A de baixo, [`MqttPublisher`], é a thread com o
//! `rumqttc`: nunca bloqueia o laço do daemon (fila limitada, o excesso é descartado), reconecta sozinha e,
//! a cada conexão, anuncia de novo a disponibilidade, a descoberta e o último estado conhecido.
//!
//! Tópicos (com `prefix = "rrv"`):
//! - `rrv/status` — `online` / `offline` (retido; `offline` é a mensagem de última vontade do broker);
//! - `rrv/<câmera>/state` — JSON retido com `status`, `online`, `recording`, `motion`, `objects`;
//! - `rrv/<câmera>/event` — um JSON por evento (não retido).
//!
//! **A URL do broker pode ter usuário e senha**: só `mqtt://host:porta` vai para o log.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use rumqttc::{Client, Event, LastWill, MqttOptions, Packet, QoS};
use serde::Deserialize;

use crate::ipc::protocol::WireEvent;

/// Espelho TOML de `[mqtt]`.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct MqttFile {
    /// Liga o MQTT. Padrão: false.
    pub enabled: Option<bool>,
    /// `mqtt://[usuario:senha@]host[:porta]` (porta 1883). Aceita `${SEGREDO}` como o resto da config.
    pub url: Option<String>,
    /// Prefixo dos tópicos. Padrão: `rrv`.
    pub prefix: Option<String>,
    /// Anuncia as câmeras ao Home Assistant. Padrão: true.
    pub discovery: Option<bool>,
    /// Prefixo da descoberta do Home Assistant. Padrão: `homeassistant`.
    pub discovery_prefix: Option<String>,
    /// Identificador do cliente. Padrão: `rrv-daemon`.
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MqttConfig {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub prefix: String,
    pub discovery: bool,
    pub discovery_prefix: String,
    pub client_id: String,
}

impl MqttFile {
    pub fn into_config(self) -> Result<MqttConfig, String> {
        let enabled = self.enabled.unwrap_or(false);
        let url = self.url.unwrap_or_default();
        let (host, port, username, password) = if url.trim().is_empty() {
            if enabled {
                return Err("mqtt.url é obrigatório com enabled = true".into());
            }
            (String::new(), 1883, None, None)
        } else {
            parse_url(&url)?
        };
        let topic_part = |name: &str, v: Option<String>, default: &str| -> Result<String, String> {
            let v = v.unwrap_or_else(|| default.to_string());
            let v = v.trim_matches('/').to_string();
            if v.is_empty() || v.contains(['+', '#', '\0']) || v.contains("//") {
                return Err(format!("mqtt.{name} '{v}' não serve como tópico"));
            }
            Ok(v)
        };
        Ok(MqttConfig {
            enabled,
            host,
            port,
            username,
            password,
            prefix: topic_part("prefix", self.prefix, "rrv")?,
            discovery: self.discovery.unwrap_or(true),
            discovery_prefix: topic_part(
                "discovery_prefix",
                self.discovery_prefix,
                "homeassistant",
            )?,
            client_id: self.client_id.unwrap_or_else(|| "rrv-daemon".into()),
        })
    }
}

type Parsed = (String, u16, Option<String>, Option<String>);

fn parse_url(url: &str) -> Result<Parsed, String> {
    let rest = if let Some(r) = url.strip_prefix("mqtt://") {
        r
    } else if url.starts_with("mqtts://") || url.starts_with("ssl://") {
        return Err(
            "mqtts:// (TLS) ainda não é suportado: use mqtt:// num broker local ou VPN".into(),
        );
    } else {
        return Err(format!(
            "mqtt.url precisa começar com mqtt:// (veio '{}')",
            safe_target(url)
        ));
    };
    let rest = rest.split(['/', '?', '#']).next().unwrap_or("");
    let (auth, hostport) = match rest.rsplit_once('@') {
        Some((a, h)) => (Some(a), h),
        None => (None, rest),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => (
            h,
            p.parse::<u16>()
                .map_err(|_| format!("mqtt.url: porta '{p}' inválida"))?,
        ),
        None => (hostport, 1883),
    };
    if host.is_empty() {
        return Err("mqtt.url sem host".into());
    }
    let (user, pass) = match auth {
        Some(a) => match a.split_once(':') {
            Some((u, p)) => (Some(u.to_string()), Some(p.to_string())),
            None => (Some(a.to_string()), None),
        },
        None => (None, None),
    };
    Ok((host.to_string(), port, user, pass))
}

/// `esquema://host[:porta]`: o que se pode mostrar sem vazar a senha.
pub fn safe_target(url: &str) -> String {
    crate::webhook::safe_target(url)
}

/// Nome de câmera → pedaço de tópico: minúsculas ASCII, o resto vira `_` (sem acentos, espaços, `/`,
/// `+` ou `#`, que quebrariam o tópico). Nomes que se igualam depois disso recebem um sufixo.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' | 'Á' | 'À' | 'Â' | 'Ã' => 'a',
            'é' | 'è' | 'ê' | 'É' | 'Ê' => 'e',
            'í' | 'ì' | 'î' | 'Í' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'Ó' | 'Ô' | 'Õ' => 'o',
            'ú' | 'ù' | 'û' | 'ü' | 'Ú' => 'u',
            'ç' | 'Ç' => 'c',
            c => c,
        };
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() { "camera".into() } else { out }
}

/// Um `slug` único por câmera, na ordem dada.
pub fn unique_slugs(names: &[String]) -> Vec<String> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    names
        .iter()
        .map(|n| {
            let base = slug(n);
            let count = seen.entry(base.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                base
            } else {
                format!("{base}_{count}")
            }
        })
        .collect()
}

/// O que se publica de cada câmera.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CameraState {
    /// `live`, `recording`, `offline`...
    pub status: String,
    pub online: bool,
    pub recording: bool,
    pub motion: bool,
    /// Objetos vistos agora (nomes COCO, sem repetição, em ordem).
    pub objects: Vec<String>,
}

impl CameraState {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "status": self.status,
            "online": self.online,
            "recording": self.recording,
            "motion": self.motion,
            "objects": self.objects,
        })
        .to_string()
    }
}

pub struct Topics<'a> {
    pub prefix: &'a str,
}

impl Topics<'_> {
    pub fn availability(&self) -> String {
        format!("{}/status", self.prefix)
    }
    pub fn state(&self, slug: &str) -> String {
        format!("{}/{slug}/state", self.prefix)
    }
    pub fn event(&self, slug: &str) -> String {
        format!("{}/{slug}/event", self.prefix)
    }
}

/// As mensagens de descoberta do Home Assistant de uma câmera: `(tópico, JSON)`. Quatro entidades num só
/// dispositivo: conectada, movimento, gravando e "objetos" (texto com o que se vê agora).
pub fn discovery_messages(cfg: &MqttConfig, slug: &str, name: &str) -> Vec<(String, String)> {
    let t = Topics {
        prefix: &cfg.prefix,
    };
    let device = serde_json::json!({
        "identifiers": [format!("rrv_{slug}")],
        "name": name,
        "manufacturer": "rust-rtsp-viewer",
        "model": "Câmera",
    });
    let base = |object: &str, label: &str| {
        serde_json::json!({
            "name": label,
            "unique_id": format!("rrv_{slug}_{object}"),
            "state_topic": t.state(slug),
            "availability_topic": t.availability(),
            "payload_available": "online",
            "payload_not_available": "offline",
            "device": device,
        })
    };
    let binary = |object: &str, label: &str, class: &str, field: &str| {
        let mut v = base(object, label);
        v["device_class"] = class.into();
        v["value_template"] = format!("{{{{ 'ON' if value_json.{field} else 'OFF' }}}}").into();
        v["payload_on"] = "ON".into();
        v["payload_off"] = "OFF".into();
        (
            format!(
                "{}/binary_sensor/rrv_{slug}/{object}/config",
                cfg.discovery_prefix
            ),
            v.to_string(),
        )
    };
    let mut objects = base("objects", "Objetos");
    objects["value_template"] =
        "{{ value_json.objects | join(', ') if value_json.objects else 'nenhum' }}".into();
    objects["icon"] = "mdi:account-search".into();
    vec![
        binary("online", "Conectada", "connectivity", "online"),
        binary("motion", "Movimento", "motion", "motion"),
        binary("recording", "Gravando", "running", "recording"),
        (
            format!("{}/sensor/rrv_{slug}/objects/config", cfg.discovery_prefix),
            objects.to_string(),
        ),
    ]
}

/// O evento em JSON para `…/event`.
pub fn event_json(ev: &WireEvent) -> String {
    serde_json::json!({
        "camera": ev.name,
        "event": ev.kind.slug(),
        "detail": ev.detail,
        "time_unix": ev.unix_secs,
    })
    .to_string()
}

const QUEUE: usize = 128;

struct Shared {
    /// O último estado publicado de cada câmera, para republicar ao reconectar e não repetir iguais.
    last: Mutex<Vec<Option<CameraState>>>,
    stop: AtomicBool,
}

pub struct MqttPublisher {
    client: Client,
    cfg: MqttConfig,
    slugs: Vec<String>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

fn announce(
    client: &Client,
    cfg: &MqttConfig,
    slugs: &[String],
    names: &[String],
    shared: &Shared,
) {
    let t = Topics {
        prefix: &cfg.prefix,
    };
    let _ = client.try_publish(t.availability(), QoS::AtLeastOnce, true, "online");
    if cfg.discovery {
        for (slug, name) in slugs.iter().zip(names) {
            for (topic, payload) in discovery_messages(cfg, slug, name) {
                let _ = client.try_publish(topic, QoS::AtLeastOnce, true, payload);
            }
        }
    }
    let last = shared.last.lock().unwrap_or_else(|e| e.into_inner());
    for (slug, state) in slugs.iter().zip(last.iter()) {
        if let Some(s) = state {
            let _ = client.try_publish(t.state(slug), QoS::AtLeastOnce, true, s.to_json());
        }
    }
}

impl MqttPublisher {
    /// Conecta (em segundo plano) e anuncia as câmeras. Nunca falha por broker fora do ar: a thread
    /// tenta de novo a cada 5 s.
    pub fn spawn(cfg: MqttConfig, names: &[String]) -> Result<Self, String> {
        let slugs = unique_slugs(names);
        let mut opts = MqttOptions::new(cfg.client_id.clone(), cfg.host.clone(), cfg.port);
        opts.set_keep_alive(Duration::from_secs(20));
        if let Some(u) = &cfg.username {
            opts.set_credentials(u.clone(), cfg.password.clone().unwrap_or_default());
        }
        let t = Topics {
            prefix: &cfg.prefix,
        };
        opts.set_last_will(LastWill::new(
            t.availability(),
            "offline",
            QoS::AtLeastOnce,
            true,
        ));
        let (client, mut connection) = Client::new(opts, QUEUE);
        let shared = Arc::new(Shared {
            last: Mutex::new(vec![None; names.len()]),
            stop: AtomicBool::new(false),
        });
        let (c, cfg2, slugs2, names2, sh) = (
            client.clone(),
            cfg.clone(),
            slugs.clone(),
            names.to_vec(),
            shared.clone(),
        );
        let target = format!("mqtt://{}:{}", cfg.host, cfg.port);
        let thread = std::thread::Builder::new()
            .name("rrv-mqtt".into())
            .spawn(move || {
                let mut was_up = false;
                for event in connection.iter() {
                    if sh.stop.load(Ordering::Relaxed) {
                        break;
                    }
                    match event {
                        Ok(Event::Incoming(Packet::ConnAck(_))) => {
                            log::info!("MQTT conectado a {target}");
                            was_up = true;
                            announce(&c, &cfg2, &slugs2, &names2, &sh);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            if was_up {
                                log::warn!("MQTT desconectado de {target}: {e}");
                                was_up = false;
                            } else {
                                log::debug!("MQTT sem conexão com {target}: {e}");
                            }
                            std::thread::sleep(Duration::from_secs(5));
                        }
                    }
                }
            })
            .map_err(|e| format!("thread do MQTT: {e}"))?;
        Ok(Self {
            client,
            cfg,
            slugs,
            shared,
            thread: Some(thread),
        })
    }

    /// Publica o estado de uma câmera se mudou desde a última vez.
    pub fn update(&self, camera: usize, state: &CameraState) {
        let Some(slug) = self.slugs.get(camera) else {
            return;
        };
        {
            let mut last = self.shared.last.lock().unwrap_or_else(|e| e.into_inner());
            if last[camera].as_ref() == Some(state) {
                return;
            }
            last[camera] = Some(state.clone());
        }
        let t = Topics {
            prefix: &self.cfg.prefix,
        };
        let _ = self
            .client
            .try_publish(t.state(slug), QoS::AtLeastOnce, true, state.to_json());
    }

    /// Publica um evento (não retido).
    pub fn event(&self, ev: &WireEvent) {
        let Some(slug) = self.slugs.get(ev.camera) else {
            return;
        };
        let t = Topics {
            prefix: &self.cfg.prefix,
        };
        let _ = self
            .client
            .try_publish(t.event(slug), QoS::AtLeastOnce, false, event_json(ev));
    }

    /// Diz `offline` e desliga (a mensagem de última vontade só vale para quedas).
    pub fn shutdown(mut self) {
        let t = Topics {
            prefix: &self.cfg.prefix,
        };
        let _ = self
            .client
            .try_publish(t.availability(), QoS::AtLeastOnce, true, "offline");
        std::thread::sleep(Duration::from_millis(200));
        self.shared.stop.store(true, Ordering::Relaxed);
        let _ = self.client.disconnect();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(toml: &str) -> Result<MqttConfig, String> {
        toml::from_str::<MqttFile>(toml).unwrap().into_config()
    }

    #[test]
    fn the_default_is_off_and_needs_no_url() {
        let c = MqttFile::default().into_config().unwrap();
        assert!(!c.enabled);
        assert_eq!(
            (c.prefix.as_str(), c.discovery_prefix.as_str()),
            ("rrv", "homeassistant")
        );
        assert!(c.discovery);
        assert!(cfg("enabled = true").unwrap_err().contains("url"));
    }

    #[test]
    fn the_url_gives_host_port_and_credentials() {
        let c = cfg("enabled = true\nurl = \"mqtt://ana:s3nha@broker.local:1884\"").unwrap();
        assert_eq!((c.host.as_str(), c.port), ("broker.local", 1884));
        assert_eq!(c.username.as_deref(), Some("ana"));
        assert_eq!(c.password.as_deref(), Some("s3nha"));
        let c = cfg("enabled = true\nurl = \"mqtt://10.0.0.5\"").unwrap();
        assert_eq!(
            (c.host.as_str(), c.port, c.username),
            ("10.0.0.5", 1883, None)
        );
        // senha com ':' e '@' codificada: o último '@' separa a credencial do host
        let c = cfg("enabled = true\nurl = \"mqtt://u:p%40ss@h:1883\"").unwrap();
        assert_eq!(c.password.as_deref(), Some("p%40ss"));
    }

    #[test]
    fn bad_urls_are_refused_without_leaking_the_password() {
        let e = cfg("enabled = true\nurl = \"http://x:segredo@h\"").unwrap_err();
        assert!(e.contains("mqtt://") && !e.contains("segredo"), "{e}");
        assert!(
            cfg("enabled = true\nurl = \"mqtts://h\"")
                .unwrap_err()
                .contains("TLS")
        );
        assert!(cfg("enabled = true\nurl = \"mqtt://h:99999\"").is_err());
        assert!(cfg("enabled = true\nurl = \"mqtt://:1883\"").is_err());
    }

    #[test]
    fn prefixes_must_be_usable_topics() {
        assert!(cfg("prefix = \"a/#\"").is_err());
        assert!(cfg("prefix = \"a+b\"").is_err());
        assert!(cfg("prefix = \"\"").is_err());
        assert_eq!(cfg("prefix = \"/casa/rrv/\"").unwrap().prefix, "casa/rrv");
    }

    #[test]
    fn camera_names_become_safe_unique_slugs() {
        assert_eq!(slug("Portão da Garagem"), "portao_da_garagem");
        assert_eq!(slug("Câmera 1 / frente #2"), "camera_1_frente_2");
        assert_eq!(slug("+++"), "camera");
        assert_eq!(
            unique_slugs(&[
                "Sala".into(),
                "sala".into(),
                "Sala!".into(),
                "Quarto".into()
            ]),
            ["sala", "sala_2", "sala_3", "quarto"]
        );
    }

    #[test]
    fn the_state_is_json_with_every_field() {
        let s = CameraState {
            status: "recording".into(),
            online: true,
            recording: true,
            motion: false,
            objects: vec!["car".into(), "person".into()],
        };
        let v: serde_json::Value = serde_json::from_str(&s.to_json()).unwrap();
        assert_eq!(v["status"], "recording");
        assert_eq!(v["online"], true);
        assert_eq!(v["motion"], false);
        assert_eq!(v["objects"], serde_json::json!(["car", "person"]));
    }

    #[test]
    fn discovery_announces_four_entities_of_one_device() {
        let c = cfg("enabled = true\nurl = \"mqtt://h\"").unwrap();
        let msgs = discovery_messages(&c, "portao", "Portão");
        assert_eq!(msgs.len(), 4);
        let topics: Vec<_> = msgs.iter().map(|m| m.0.as_str()).collect();
        assert!(topics.contains(&"homeassistant/binary_sensor/rrv_portao/motion/config"));
        assert!(topics.contains(&"homeassistant/sensor/rrv_portao/objects/config"));
        let mut ids = std::collections::HashSet::new();
        for (_, body) in &msgs {
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["state_topic"], "rrv/portao/state");
            assert_eq!(v["availability_topic"], "rrv/status");
            assert_eq!(v["device"]["identifiers"][0], "rrv_portao");
            assert_eq!(v["device"]["name"], "Portão");
            assert!(
                ids.insert(v["unique_id"].as_str().unwrap().to_string()),
                "unique_id repetido"
            );
        }
        let motion: serde_json::Value = serde_json::from_str(&msgs[1].1).unwrap();
        assert_eq!(motion["device_class"], "motion");
        assert!(
            motion["value_template"]
                .as_str()
                .unwrap()
                .contains("value_json.motion")
        );
    }

    #[test]
    fn topics_follow_the_prefix() {
        let t = Topics { prefix: "casa/rrv" };
        assert_eq!(t.availability(), "casa/rrv/status");
        assert_eq!(t.state("sala"), "casa/rrv/sala/state");
        assert_eq!(t.event("sala"), "casa/rrv/sala/event");
    }

    #[test]
    fn an_event_becomes_a_small_json() {
        let ev = WireEvent {
            camera: 0,
            name: "Portão".into(),
            kind: crate::domain::timeline::EventType::Detection,
            detail: Some("person 90%".into()),
            notification: None,
            unix_secs: 1_700_000_000,
        };
        let v: serde_json::Value = serde_json::from_str(&event_json(&ev)).unwrap();
        assert_eq!(v["event"], "detection");
        assert_eq!(v["camera"], "Portão");
        assert_eq!(v["detail"], "person 90%");
        assert_eq!(v["time_unix"], 1_700_000_000);
    }
}
