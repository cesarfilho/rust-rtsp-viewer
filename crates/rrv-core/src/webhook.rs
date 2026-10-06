//! O aviso com a janela fechada (plano 2.5.11): o daemon entrega as
//! notificações que a política do motor já decidiu (tipo de evento e cooldown)
//! a uma URL, para Home Assistant, ntfy, n8n, Node-RED, etc.
//!
//! Usa o `curl` como subprocesso, numa thread própria: HTTPS, proxies e
//! certificados vêm prontos, sem dependência nova, e um destino lento ou fora do
//! ar nunca trava o daemon (a fila é limitada; o excesso é descartado).
//!
//! **A URL nunca vai inteira para o log**: o token costuma estar nela (o tópico
//! do ntfy, o id do webhook do Home Assistant). Só o `esquema://host` aparece.

use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::thread::JoinHandle;

use serde::Deserialize;

use crate::domain::timeline::EventType;
use crate::ipc::protocol::WireEvent;

/// Quantos avisos esperam na fila antes de se descartar o excesso.
const QUEUE: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookFormat {
    /// `{"camera":…, "event":"motion", …}`: Home Assistant, n8n, Node-RED.
    Json,
    /// O corpo é a mensagem; título e etiqueta vão em cabeçalhos (ntfy).
    Ntfy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebhookConfig {
    pub url: String,
    pub format: WebhookFormat,
    pub timeout_secs: u64,
}

/// Espelho TOML de `[webhook]`.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct WebhookFile {
    /// Destino do POST (`http://` ou `https://`).
    pub url: Option<String>,
    /// `json` (padrão) ou `ntfy`.
    pub format: Option<String>,
    /// Prazo por envio, em segundos (1–60). Padrão: 5.
    pub timeout_secs: Option<u64>,
}

impl WebhookFile {
    /// `Ok(None)` quando a seção existe mas está vazia; erro com a mensagem do
    /// campo quando algo não serve.
    pub fn into_config(self) -> Result<Option<WebhookConfig>, String> {
        let Some(url) = self.url.filter(|u| !u.trim().is_empty()) else {
            return Ok(None);
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(format!(
                "webhook.url precisa começar com http:// ou https:// (veio '{}')",
                safe_target(&url)
            ));
        }
        let format = match self.format.as_deref().unwrap_or("json") {
            "json" => WebhookFormat::Json,
            "ntfy" => WebhookFormat::Ntfy,
            other => {
                return Err(format!(
                    "webhook.format '{other}' desconhecido (json ou ntfy)"
                ));
            }
        };
        let timeout_secs = self.timeout_secs.unwrap_or(5);
        if !(1..=60).contains(&timeout_secs) {
            return Err(format!("webhook.timeout_secs {timeout_secs} fora de 1–60"));
        }
        Ok(Some(WebhookConfig {
            url,
            format,
            timeout_secs,
        }))
    }
}

/// `esquema://host[:porta]`: o que se pode mostrar sem vazar o token.
pub fn safe_target(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // tira `usuario:senha@`
    let host = authority.rsplit('@').next().unwrap_or(authority);
    if scheme.is_empty() {
        host.to_string()
    } else {
        format!("{scheme}://{host}")
    }
}

fn kind_slug(kind: EventType) -> &'static str {
    match kind {
        EventType::Motion => "motion",
        EventType::RecordingStart => "recording_start",
        EventType::RecordingStop => "recording_stop",
        EventType::Offline => "offline",
        EventType::Online => "online",
        EventType::Snapshot => "snapshot",
        EventType::DiskLow => "disk_low",
        EventType::Detection => "detection",
    }
}

/// Escapa um texto para dentro de uma string JSON.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Base64 padrão (para cabeçalhos com acento, RFC 2047).
fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Um valor de cabeçalho: ASCII puro passa direto; com acento vira
/// `=?UTF-8?B?…?=` (o que o ntfy aceita).
fn header_value(s: &str) -> String {
    if s.is_ascii() {
        s.to_string()
    } else {
        format!("=?UTF-8?B?{}?=", base64(s.as_bytes()))
    }
}

/// O que enviar para um evento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub body: String,
    pub headers: Vec<(String, String)>,
}

/// Monta o pedido de um evento. `None` quando o motor não mandou avisar (a
/// política de tipo e cooldown é dele).
pub fn build_request(config: &WebhookConfig, event: &WireEvent) -> Option<Request> {
    let (title, message) = event.notification.as_ref()?;
    Some(match config.format {
        WebhookFormat::Json => Request {
            body: format!(
                "{{\"camera\":{},\"event\":{},\"label\":{},\"detail\":{},\"title\":{},\"message\":{},\"time_unix\":{}}}",
                json_str(&event.name),
                json_str(kind_slug(event.kind)),
                json_str(event.kind.label()),
                event.detail.as_deref().map_or("null".to_string(), json_str),
                json_str(title),
                json_str(message),
                event.unix_secs
            ),
            headers: vec![("Content-Type".into(), "application/json".into())],
        },
        WebhookFormat::Ntfy => Request {
            body: message.clone(),
            headers: vec![
                ("Title".into(), header_value(title)),
                ("Tags".into(), kind_slug(event.kind).into()),
                ("Content-Type".into(), "text/plain; charset=utf-8".into()),
            ],
        },
    })
}

/// Entrega um pedido com o `curl`. `Err` com o motivo (sem a URL).
fn deliver(config: &WebhookConfig, req: &Request) -> Result<(), String> {
    let mut cmd = Command::new("curl");
    cmd.args([
        "-sS",
        "--fail",
        "--max-time",
        &config.timeout_secs.to_string(),
    ])
    .args(["-X", "POST", "--data-binary", "@-"]);
    for (k, v) in &req.headers {
        cmd.args(["-H", &format!("{k}: {v}")]);
    }
    cmd.arg(&config.url)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("não consegui executar o curl ({e}); ele está instalado?"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(req.body.as_bytes());
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        // A mensagem do curl pode conter a URL: tira antes de devolver.
        let stderr =
            String::from_utf8_lossy(&out.stderr).replace(&config.url, &safe_target(&config.url));
        Err(stderr.trim().to_string())
    }
}

/// O envio em segundo plano.
pub struct Webhook {
    config: WebhookConfig,
    /// Ligado ao desligar: o que ainda está na fila é descartado.
    closing: Arc<AtomicBool>,
    tx: Option<SyncSender<Request>>,
    thread: Option<JoinHandle<()>>,
}

impl Webhook {
    pub fn spawn(config: WebhookConfig) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Request>(QUEUE);
        let cfg = config.clone();
        let closing = Arc::new(AtomicBool::new(false));
        let flag = closing.clone();
        let thread = std::thread::Builder::new()
            .name("rrv-webhook".into())
            .spawn(move || {
                for req in rx {
                    if flag.load(Ordering::Relaxed) {
                        continue; // desligando: descarta, não entrega
                    }
                    if let Err(why) = deliver(&cfg, &req) {
                        log::warn!("webhook para {} falhou: {why}", safe_target(&cfg.url));
                    }
                }
            })
            .ok();
        Self {
            config,
            closing,
            tx: Some(tx),
            thread,
        }
    }

    /// Enfileira o aviso de um evento (se o motor mandou avisar). Nunca bloqueia:
    /// com a fila cheia o excesso é descartado.
    pub fn send(&self, event: &WireEvent) {
        let Some(req) = build_request(&self.config, event) else {
            return;
        };
        if let Some(tx) = &self.tx
            && let Err(TrySendError::Full(_)) = tx.try_send(req)
        {
            log::warn!(
                "webhook para {} atrasado: aviso descartado",
                safe_target(&self.config.url)
            );
        }
    }

    pub fn target(&self) -> String {
        safe_target(&self.config.url)
    }
}

impl Drop for Webhook {
    fn drop(&mut self) {
        // Descarta a fila (senão um destino fora do ar atrasaria o SIGTERM por
        // fila × prazo) e espera só o envio que já estiver em andamento.
        self.closing.store(true, Ordering::Relaxed);
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(url: &str) -> WebhookFile {
        WebhookFile {
            url: Some(url.into()),
            ..Default::default()
        }
    }

    fn event(notification: bool) -> WireEvent {
        WireEvent {
            camera: 0,
            name: "Portão".into(),
            kind: EventType::Motion,
            detail: Some("3% do quadro".into()),
            notification: notification
                .then(|| ("Movimento: Portão".into(), "3% \"do\" quadro".into())),
            unix_secs: 1_700_000_000,
        }
    }

    fn cfg(format: WebhookFormat) -> WebhookConfig {
        WebhookConfig {
            url: "https://x.example/hook/SEGREDO".into(),
            format,
            timeout_secs: 5,
        }
    }

    #[test]
    fn an_empty_section_means_no_webhook() {
        assert_eq!(WebhookFile::default().into_config().unwrap(), None);
        assert_eq!(file("   ").into_config().unwrap(), None);
    }

    #[test]
    fn the_config_is_validated_by_field() {
        let ok = file("https://ntfy.sh/meu-topico")
            .into_config()
            .unwrap()
            .unwrap();
        assert_eq!(ok.format, WebhookFormat::Json);
        assert_eq!(ok.timeout_secs, 5);
        let e = file("ftp://x").into_config().unwrap_err();
        assert!(e.contains("webhook.url") && e.contains("http"), "{e}");
        let bad_fmt = WebhookFile {
            format: Some("xml".into()),
            ..file("http://x")
        };
        assert!(
            bad_fmt
                .into_config()
                .unwrap_err()
                .contains("webhook.format")
        );
        let bad_t = WebhookFile {
            timeout_secs: Some(0),
            ..file("http://x")
        };
        assert!(bad_t.into_config().unwrap_err().contains("timeout_secs"));
    }

    #[test]
    fn the_url_never_leaks_beyond_scheme_and_host() {
        assert_eq!(
            safe_target("https://ntfy.sh/segredo-do-topico?auth=abc"),
            "https://ntfy.sh"
        );
        assert_eq!(
            safe_target("http://user:senha@ha.local:8123/api/webhook/ID"),
            "http://ha.local:8123"
        );
        let err = file("ftp://tok:en@h/x").into_config().unwrap_err();
        assert!(!err.contains("tok") && !err.contains("en@"), "{err}");
    }

    #[test]
    fn json_carries_what_a_receiver_needs_and_escapes_text() {
        let req = build_request(&cfg(WebhookFormat::Json), &event(true)).unwrap();
        assert!(req.body.contains("\"camera\":\"Portão\""), "{}", req.body);
        assert!(req.body.contains("\"event\":\"motion\""));
        assert!(req.body.contains("\"detail\":\"3% do quadro\""));
        assert!(req.body.contains("\"time_unix\":1700000000"));
        assert!(
            req.body.contains("3% \\\"do\\\" quadro"),
            "aspas escapadas: {}",
            req.body
        );
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v == "application/json")
        );
    }

    #[test]
    fn the_body_is_real_json_with_the_expected_fields() {
        let body = build_request(&cfg(WebhookFormat::Json), &event(true))
            .unwrap()
            .body;
        let v: serde_json::Value = serde_json::from_str(&body).expect("JSON inválido");
        assert_eq!(v["camera"], "Portão");
        assert_eq!(v["event"], "motion");
        assert_eq!(v["label"], "Motion");
        assert_eq!(v["detail"], "3% do quadro");
        assert_eq!(v["title"], "Movimento: Portão");
        assert_eq!(v["message"], "3% \"do\" quadro");
        assert_eq!(v["time_unix"], 1_700_000_000u64);
        // sem detalhe, o campo é null (não some)
        let mut e = event(true);
        e.detail = None;
        let v: serde_json::Value =
            serde_json::from_str(&build_request(&cfg(WebhookFormat::Json), &e).unwrap().body)
                .unwrap();
        assert!(v["detail"].is_null());
    }

    #[test]
    fn ntfy_puts_the_text_in_the_body_and_encodes_accents_in_the_header() {
        let req = build_request(&cfg(WebhookFormat::Ntfy), &event(true)).unwrap();
        assert_eq!(req.body, "3% \"do\" quadro");
        let title = &req.headers.iter().find(|(k, _)| k == "Title").unwrap().1;
        assert!(
            title.starts_with("=?UTF-8?B?") && title.ends_with("?="),
            "{title}"
        );
        assert!(
            req.headers
                .iter()
                .any(|(k, v)| k == "Tags" && v == "motion")
        );
    }

    #[test]
    fn ascii_headers_are_sent_as_they_are() {
        assert_eq!(header_value("Camera 1"), "Camera 1");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn events_the_engine_did_not_flag_are_not_sent() {
        assert_eq!(
            build_request(&cfg(WebhookFormat::Json), &event(false)),
            None
        );
    }
}
