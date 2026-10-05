//! O protocolo entre a janela (ou `rrvctl`) e o daemon (ADR 0010).
//!
//! Uma mensagem JSON por linha, sobre um socket Unix. A primeira mensagem do
//! cliente é sempre `Hello`; o daemon recusa versões que não conhece. O
//! protocolo é versionado para a janela e o daemon poderem ser atualizados em
//! momentos diferentes (a janela é nativa; o daemon roda em contêiner).

use serde::{Deserialize, Serialize};

use crate::domain::timeline::EventType;
use crate::domain::zones::MotionZoneFile;
use crate::engine::EngineEvent;

/// Versão do protocolo. Sobe quando uma mudança não é compatível.
pub const PROTOCOL_VERSION: u32 = 1;

/// Pedido do cliente. `cmd` identifica a variante no JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Primeira mensagem de toda conexão.
    Hello { protocol: u32 },
    /// Estado de todas as câmeras.
    Status,
    /// Passa a receber os eventos do motor (`ServerMessage::Event`).
    Subscribe,
    /// Liga ou desliga a gravação de uma câmera (índice do `Status`).
    ToggleRecording { camera: usize },
    /// Liga ou desliga uma câmera.
    SetCameraEnabled { camera: usize, enabled: bool },
    /// Zonas de movimento de uma câmera.
    GetZones { camera: usize },
    /// Substitui as zonas de uma câmera (lista vazia remove) e persiste.
    SetZones {
        camera: usize,
        zones: Vec<MotionZoneFile>,
    },
}

/// Resposta do daemon a um pedido.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    Hello {
        protocol: u32,
        server: String,
    },
    Status {
        cameras: Vec<CameraInfo>,
    },
    Recording {
        camera: usize,
        recording: bool,
    },
    Zones {
        camera: usize,
        zones: Vec<MotionZoneFile>,
    },
    Subscribed,
    Ok,
    Error {
        message: String,
    },
}

/// O que o daemon envia: respostas e, depois de `Subscribe`, eventos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Response(Response),
    Event(WireEvent),
}

/// Uma câmera como o cliente a vê.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraInfo {
    pub index: usize,
    pub name: String,
    /// `connecting`, `live`, `recording`, `offline`, `reconnecting`, `paused`, `disabled`.
    pub status: String,
    pub enabled: bool,
    pub recording: bool,
    pub motion: bool,
    /// `main` ou `sub`.
    pub stream: String,
}

/// Um evento do motor, no formato do fio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireEvent {
    pub camera: usize,
    pub name: String,
    pub kind: EventType,
    pub detail: Option<String>,
    /// `(título, corpo)` quando a política de notificação manda avisar.
    pub notification: Option<(String, String)>,
    pub unix_secs: u64,
}

impl WireEvent {
    pub fn from_engine(event: &EngineEvent, name: &str, unix_secs: u64) -> Self {
        Self {
            camera: event.camera,
            name: name.to_string(),
            kind: event.kind,
            detail: event.detail.clone(),
            notification: event.notification.clone(),
            unix_secs,
        }
    }
}

/// Codifica uma mensagem como uma linha JSON (com `\n`).
pub fn encode_line<T: Serialize>(message: &T) -> String {
    let mut line = serde_json::to_string(message).expect("as mensagens do protocolo serializam");
    line.push('\n');
    line
}

/// Decodifica uma linha JSON.
pub fn decode_line<'a, T: Deserialize<'a>>(line: &'a str) -> Result<T, String> {
    serde_json::from_str(line.trim_end()).map_err(|e| format!("mensagem inválida: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_as_single_json_lines() {
        let reqs = [
            Request::Hello {
                protocol: PROTOCOL_VERSION,
            },
            Request::Status,
            Request::Subscribe,
            Request::ToggleRecording { camera: 2 },
            Request::SetCameraEnabled {
                camera: 0,
                enabled: false,
            },
            Request::GetZones { camera: 1 },
            Request::SetZones {
                camera: 1,
                zones: vec![],
            },
        ];
        for r in reqs {
            let line = encode_line(&r);
            assert!(line.ends_with('\n') && !line[..line.len() - 1].contains('\n'));
            assert_eq!(decode_line::<Request>(&line).unwrap(), r);
        }
    }

    #[test]
    fn the_wire_format_is_stable() {
        // O formato JSON é o contrato entre versões: este teste o fixa.
        assert_eq!(
            encode_line(&Request::ToggleRecording { camera: 3 }),
            "{\"cmd\":\"toggle_recording\",\"camera\":3}\n"
        );
        assert_eq!(
            encode_line(&Request::Hello { protocol: 1 }),
            "{\"cmd\":\"hello\",\"protocol\":1}\n"
        );
        assert_eq!(
            encode_line(&ServerMessage::Response(Response::Ok)),
            "{\"type\":\"response\",\"kind\":\"ok\"}\n"
        );
    }

    #[test]
    fn server_messages_distinguish_responses_from_events() {
        let ev = ServerMessage::Event(WireEvent {
            camera: 0,
            name: "Portão".into(),
            kind: EventType::Motion,
            detail: Some("3% do quadro".into()),
            notification: None,
            unix_secs: 1_700_000_000,
        });
        let line = encode_line(&ev);
        assert!(line.contains("\"type\":\"event\""), "{line}");
        assert_eq!(decode_line::<ServerMessage>(&line).unwrap(), ev);
        let r = ServerMessage::Response(Response::Error {
            message: "x".into(),
        });
        assert!(encode_line(&r).contains("\"type\":\"response\""));
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_line::<Request>("isto não é json").is_err());
        assert!(decode_line::<Request>("{\"cmd\":\"nao_existe\"}").is_err());
    }
}
