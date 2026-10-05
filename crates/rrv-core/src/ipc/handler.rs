//! O que cada pedido faz no motor. Puro: recebe o `Engine` e devolve a resposta,
//! sem socket nem thread, então se testa sem rede. O servidor só entrega os
//! pedidos aqui, no laço principal do daemon (o `Engine` é de uma thread só).

use crate::domain::camera_status::CameraStatus;
use crate::domain::multi_stream::StreamQuality;
use crate::domain::zones::{MotionZoneFile, ZoneConfig};
use crate::engine::Engine;
use crate::infrastructure::zone_state::{self, ZonesFile};

use super::protocol::{CameraInfo, PROTOCOL_VERSION, Request, Response};

/// O estado que o tratador altera além do motor: as zonas persistidas.
pub struct Host<'a> {
    pub engine: &'a mut Engine,
    pub zones_file: &'a mut ZonesFile,
    /// Persistir de verdade (desligado nos testes, para não tocar no disco).
    pub persist: bool,
}

fn status_name(s: &CameraStatus) -> &'static str {
    match s {
        CameraStatus::Live => "live",
        CameraStatus::Offline => "offline",
        CameraStatus::Reconnecting => "reconnecting",
        CameraStatus::Recording => "recording",
        CameraStatus::Disabled => "disabled",
        CameraStatus::Connecting => "connecting",
        CameraStatus::Paused => "paused",
    }
}

fn error(message: impl Into<String>) -> Response {
    Response::Error {
        message: message.into(),
    }
}

/// Fotografia das câmeras.
pub fn camera_infos(engine: &Engine) -> Vec<CameraInfo> {
    (0..engine.camera_count())
        .map(|i| {
            let stream = engine.bridges[i]
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .metrics()
                .snapshot_stream_info();
            CameraInfo {
                index: i,
                name: engine.names[i].clone(),
                status: status_name(&engine.status[i]).into(),
                enabled: engine.camera_enabled[i],
                recording: engine.status[i] == CameraStatus::Recording
                    || engine.bridges[i]
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_recording(),
                motion: engine.motion_active[i],
                stream: match engine.stream_quality[i] {
                    StreamQuality::Main => "main",
                    StreamQuality::Sub => "sub",
                }
                .into(),
                decoder: stream.decoder,
                decoder_hw: stream.decoder_hw,
            }
        })
        .collect()
}

/// Executa um pedido. `Hello` e `Subscribe` são da conexão, não do motor, e o
/// servidor os trata antes de chegar aqui; recebê-los aqui é um erro de uso.
pub fn apply(host: &mut Host<'_>, request: &Request) -> Response {
    let count = host.engine.camera_count();
    let check = |camera: usize| -> Result<(), Response> {
        if camera < count {
            Ok(())
        } else {
            Err(error(format!(
                "câmera {camera} não existe (há {count} câmera(s))"
            )))
        }
    };
    match request {
        Request::Hello { .. } | Request::Subscribe => {
            error("Hello e Subscribe são tratados pela conexão")
        }
        Request::Status => Response::Status {
            cameras: camera_infos(host.engine),
        },
        Request::ToggleRecording { camera } => {
            if let Err(e) = check(*camera) {
                return e;
            }
            if !host.engine.camera_enabled[*camera] || !host.engine.active_stream[*camera] {
                return error("a câmera não está rodando");
            }
            match host.engine.toggle_recording(*camera) {
                Ok(recording) => {
                    // Uma gravação manual não é do gatilho de movimento: ele
                    // nunca a para.
                    host.engine.auto_recording[*camera] = false;
                    Response::Recording {
                        camera: *camera,
                        recording,
                    }
                }
                Err(e) => error(e),
            }
        }
        Request::SetCameraEnabled { camera, enabled } => {
            if let Err(e) = check(*camera) {
                return e;
            }
            host.engine.set_camera_enabled(*camera, *enabled);
            Response::Ok
        }
        Request::GetZones { camera } => {
            if let Err(e) = check(*camera) {
                return e;
            }
            Response::Zones {
                camera: *camera,
                zones: host.engine.zones[*camera]
                    .zones
                    .iter()
                    .map(MotionZoneFile::from_zone)
                    .collect(),
            }
        }
        Request::SetZones { camera, zones } => {
            if let Err(e) = check(*camera) {
                return e;
            }
            let config = ZoneConfig {
                zones: zones
                    .iter()
                    .cloned()
                    .map(MotionZoneFile::into_zone)
                    .collect(),
            };
            if let Some(bad) = config.zones.iter().find(|z| !z.is_valid()) {
                return error(format!(
                    "a zona '{}' é inválida (precisa de 3+ vértices em 0..1)",
                    bad.name
                ));
            }
            // As zonas são guardadas pelo nome da câmera, nunca pela URL (que
            // carrega a senha).
            let name = host.engine.names[*camera].clone();
            host.zones_file.set(&name, &config);
            host.engine.zones[*camera] = config;
            host.engine.prev_motion_frames[*camera] = None;
            if host.persist {
                zone_state::save(host.zones_file);
            }
            Response::Ok
        }
    }
}

/// A resposta a um `Hello`: confere a versão.
pub fn hello(protocol: u32) -> Response {
    if protocol == PROTOCOL_VERSION {
        Response::Hello {
            protocol: PROTOCOL_VERSION,
            server: format!("rrv-daemon {}", env!("CARGO_PKG_VERSION")),
        }
    } else {
        error(format!(
            "versão do protocolo {protocol} não suportada (o daemon fala {PROTOCOL_VERSION})"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CameraConfig, LogsConfigFile};
    use crate::domain::motion::MotionConfig;
    use crate::domain::notify::NotifyConfig;
    use crate::domain::recording::RecordingConfig;
    use crate::domain::zones::PointFile;
    use crate::engine::EngineSettings;
    use std::time::Duration;

    fn engine(n: usize) -> Engine {
        let cams: Vec<CameraConfig> = (0..n)
            .map(|i| toml::from_str(&format!("url = \"rtsp://h/{i}\"\nname = \"cam{i}\"")).unwrap())
            .collect();
        let dir = std::env::temp_dir().join(format!("rrv-ipc-h-{}", std::process::id()));
        Engine::new(EngineSettings {
            cameras: &cams,
            recording: &RecordingConfig::default(),
            motion: MotionConfig::default(),
            notify: NotifyConfig::default(),
            logs: &LogsConfigFile {
                dir: Some(dir),
                ..LogsConfigFile::default()
            },
            pause_hidden: true,
            stagger: Duration::from_millis(100),
            zones: &ZonesFile::default(),
        })
    }

    fn run(engine: &mut Engine, zones: &mut ZonesFile, r: &Request) -> Response {
        apply(
            &mut Host {
                engine,
                zones_file: zones,
                persist: false,
            },
            r,
        )
    }

    fn triangle(name: &str) -> MotionZoneFile {
        MotionZoneFile {
            name: name.into(),
            vertices: vec![
                PointFile { x: 0.1, y: 0.1 },
                PointFile { x: 0.9, y: 0.1 },
                PointFile { x: 0.5, y: 0.9 },
            ],
            enabled: Some(true),
        }
    }

    #[test]
    fn status_lists_every_camera() {
        let mut e = engine(2);
        let mut z = ZonesFile::default();
        let Response::Status { cameras } = run(&mut e, &mut z, &Request::Status) else {
            panic!("esperava Status");
        };
        assert_eq!(cameras.len(), 2);
        assert_eq!(cameras[1].name, "cam1");
        assert_eq!(cameras[1].status, "connecting");
        assert_eq!(cameras[1].stream, "main");
        assert!(cameras[1].enabled && !cameras[1].recording && !cameras[1].motion);
        assert_eq!(cameras[1].decoder, None, "ainda não subiu");
        assert!(!cameras[1].decoder_hw);
    }

    #[test]
    fn an_unknown_camera_is_a_clear_error() {
        let mut e = engine(1);
        let mut z = ZonesFile::default();
        for r in [
            Request::ToggleRecording { camera: 5 },
            Request::GetZones { camera: 5 },
            Request::SetCameraEnabled {
                camera: 5,
                enabled: false,
            },
        ] {
            let Response::Error { message } = run(&mut e, &mut z, &r) else {
                panic!("esperava erro para {r:?}");
            };
            assert!(message.contains("câmera 5 não existe"), "{message}");
        }
    }

    #[test]
    fn toggling_a_camera_that_is_not_running_is_refused() {
        let mut e = engine(1);
        let mut z = ZonesFile::default();
        let Response::Error { message } =
            run(&mut e, &mut z, &Request::ToggleRecording { camera: 0 })
        else {
            panic!("esperava erro");
        };
        assert!(message.contains("não está rodando"), "{message}");
    }

    #[test]
    fn disabling_and_enabling_a_camera_goes_through_the_engine() {
        let mut e = engine(2);
        let mut z = ZonesFile::default();
        let off = Request::SetCameraEnabled {
            camera: 1,
            enabled: false,
        };
        assert_eq!(run(&mut e, &mut z, &off), Response::Ok);
        assert!(!e.camera_enabled[1]);
        assert_eq!(e.status[1], CameraStatus::Disabled);
        let Response::Status { cameras } = run(&mut e, &mut z, &Request::Status) else {
            panic!()
        };
        assert_eq!(cameras[1].status, "disabled");
        assert!(!cameras[1].enabled);
    }

    #[test]
    fn zones_round_trip_and_are_stored_by_camera_name() {
        let mut e = engine(2);
        let mut z = ZonesFile::default();
        let set = Request::SetZones {
            camera: 1,
            zones: vec![triangle("porta")],
        };
        assert_eq!(run(&mut e, &mut z, &set), Response::Ok);
        assert!(e.zones[1].has_active(), "o motor passa a usar a zona");
        assert!(
            z.cameras.contains_key("cam1"),
            "guardada pelo nome, não pela URL"
        );
        assert!(!z.cameras.keys().any(|k| k.contains("rtsp")));

        let Response::Zones { zones, .. } = run(&mut e, &mut z, &Request::GetZones { camera: 1 })
        else {
            panic!()
        };
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].name, "porta");

        // lista vazia remove
        let clear = Request::SetZones {
            camera: 1,
            zones: vec![],
        };
        assert_eq!(run(&mut e, &mut z, &clear), Response::Ok);
        assert!(!e.zones[1].has_active());
        assert!(z.cameras.is_empty());
    }

    #[test]
    fn an_invalid_zone_is_rejected_and_changes_nothing() {
        let mut e = engine(1);
        let mut z = ZonesFile::default();
        let mut bad = triangle("ruim");
        bad.vertices.truncate(2); // só 2 vértices
        let Response::Error { message } = run(
            &mut e,
            &mut z,
            &Request::SetZones {
                camera: 0,
                zones: vec![bad],
            },
        ) else {
            panic!("esperava erro");
        };
        assert!(message.contains("inválida"), "{message}");
        assert!(!e.zones[0].has_active());
        assert!(z.cameras.is_empty());
    }

    #[test]
    fn hello_checks_the_protocol_version() {
        assert!(matches!(
            hello(PROTOCOL_VERSION),
            Response::Hello { protocol, .. } if protocol == PROTOCOL_VERSION
        ));
        let Response::Error { message } = hello(999) else {
            panic!()
        };
        assert!(message.contains("não suportada"), "{message}");
    }
}
