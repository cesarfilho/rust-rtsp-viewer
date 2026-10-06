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
    /// Segmentos gravados e eventos num intervalo (Unix ms). `camera` é o *nome*
    /// da câmera; sem ele, todas.
    History {
        camera: Option<String>,
        from_ms: i64,
        to_ms: i64,
    },
    /// Exporta um clipe (`.mp4`, sem reencode) de uma câmera e um intervalo (Unix ms) para
    /// a pasta `exports/` das gravações.
    ExportClip {
        camera: String,
        from_ms: i64,
        to_ms: i64,
    },
    /// Protege (ou solta) um segmento: a retenção nunca apaga um protegido.
    SetProtected { segment_id: i64, protected: bool },
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
    History {
        segments: Vec<SegmentInfo>,
        events: Vec<HistoryEvent>,
        /// `true` se o intervalo tinha mais que o limite por resposta.
        truncated: bool,
    },
    Exported {
        /// Caminho do clipe relativo à pasta de gravações.
        file: String,
        bytes: u64,
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

/// Um segmento gravado. O caminho do arquivo é do daemon e não sai dele.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentInfo {
    pub id: i64,
    pub camera: String,
    pub ts_start: i64,
    /// `None` enquanto o segmento está aberto (ainda gravando).
    pub ts_end: Option<i64>,
    pub bytes: i64,
    pub has_motion: bool,
    pub protected: bool,
    /// `motion` ou `manual`.
    pub mode: String,
    /// Arquivo do segmento **relativo à pasta de gravações**: o cliente o junta à sua
    /// própria pasta (a do host, quando o daemon roda em contêiner).
    #[serde(default)]
    pub file: String,
}

/// Um evento do histórico persistente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub id: i64,
    pub camera: String,
    pub ts: i64,
    /// `motion`, `recording_start`, `recording_stop`, `offline`, `online`, `snapshot`.
    pub kind: String,
    pub label: String,
    pub segment_id: Option<i64>,
    /// Confiança (0–1) de uma detecção.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// Caixa `[x, y, w, h]` normalizada ao quadro, nas detecções.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[f32; 4]>,
    /// Zona de movimento em que o objeto estava, nas detecções.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
}

/// Um objeto reconhecido agora na câmera (caixa normalizada ao quadro).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireBox {
    /// Nome COCO em inglês (`person`, `car`...).
    pub label: String,
    pub score: f32,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Uma câmera como o cliente a vê.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    /// O decodificador em uso (`vah264dec`, `avdec_h264`…); vazio até a câmera
    /// subir. Campo aditivo: um cliente antigo, que não o conhece, continua
    /// funcionando (por isso o `default`).
    #[serde(default)]
    pub decoder: Option<String>,
    /// O decodificador roda na GPU (ou num bloco de função fixa).
    #[serde(default)]
    pub decoder_hw: bool,
    /// Resolução do vídeo (0 até a câmera subir).
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    /// Quadros por segundo medidos agora.
    #[serde(default)]
    pub fps: f64,
    /// Taxa do fluxo comprimido, em kbit/s.
    #[serde(default)]
    pub bitrate_kbps: u64,
    /// Codec do fluxo (`H264`, `H265`…).
    #[serde(default)]
    pub codec: Option<String>,
    /// Objetos reconhecidos no último quadro analisado (detecção de objetos ligada); vazio se
    /// não há nenhum. Aditivo: um cliente antigo ignora o campo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detections: Vec<WireBox>,
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
    fn camera_info_from_an_older_daemon_still_parses() {
        // sem os campos `decoder` e `decoder_hw`, que são posteriores
        let old = "{\"index\":0,\"name\":\"Portão\",\"status\":\"live\",\"enabled\":true,\
                   \"recording\":false,\"motion\":false,\"stream\":\"main\"}";
        let info: CameraInfo = serde_json::from_str(old).unwrap();
        assert_eq!(info.decoder, None);
        assert!(!info.decoder_hw);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(decode_line::<Request>("isto não é json").is_err());
        assert!(decode_line::<Request>("{\"cmd\":\"nao_existe\"}").is_err());
    }
}
