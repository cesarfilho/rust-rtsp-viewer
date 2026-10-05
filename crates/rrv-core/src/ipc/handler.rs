//! O que cada pedido faz no motor. Puro: recebe o `Engine` e devolve a resposta,
//! sem socket nem thread, então se testa sem rede. O servidor só entrega os
//! pedidos aqui, no laço principal do daemon (o `Engine` é de uma thread só).

use crate::domain::camera_status::CameraStatus;
use crate::domain::multi_stream::StreamQuality;
use crate::domain::zones::{MotionZoneFile, ZoneConfig};
use crate::engine::Engine;
use crate::engine::clip::{ClipSource, export_clip};
use crate::infrastructure::zone_state::{self, ZonesFile};

use crate::infrastructure::store::Store;

use super::protocol::{CameraInfo, HistoryEvent, PROTOCOL_VERSION, Request, Response, SegmentInfo};

/// O estado que o tratador altera além do motor: as zonas persistidas.
pub struct Host<'a> {
    pub engine: &'a mut Engine,
    pub zones_file: &'a mut ZonesFile,
    /// Persistir de verdade (desligado nos testes, para não tocar no disco).
    pub persist: bool,
    /// O histórico (`None`: o daemon está sem banco).
    pub history: Option<&'a Store>,
    /// A pasta de gravações do daemon (para os nomes relativos e para `exports/`).
    pub recordings: Option<&'a std::path::Path>,
}

/// Teto de um clipe exportado: o pedido roda no laço do daemon.
const MAX_CLIP_MS: i64 = 30 * 60 * 1000;

/// Teto de linhas por resposta de histórico (cada lista).
const HISTORY_LIMIT: usize = 5000;

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

/// `path` relativo a `base` (ou só o nome do arquivo, se não estiver dentro dela).
fn relative(base: Option<&std::path::Path>, path: &str) -> String {
    let p = std::path::Path::new(path);
    base.and_then(|b| p.strip_prefix(b).ok())
        .or_else(|| p.file_name().map(std::path::Path::new))
        .map(|r| r.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Fotografia das câmeras.
pub fn camera_infos(engine: &Engine) -> Vec<CameraInfo> {
    (0..engine.camera_count())
        .map(|i| {
            let (stream, fps, bitrate_kbps) = {
                let b = engine.bridges[i].lock().unwrap_or_else(|e| e.into_inner());
                (
                    b.metrics().snapshot_stream_info(),
                    b.last_fps(),
                    b.last_bitrate_kbps(),
                )
            };
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
                width: stream.width.unwrap_or(0).max(0) as u32,
                height: stream.height.unwrap_or(0).max(0) as u32,
                fps,
                bitrate_kbps,
                codec: stream.codec,
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
        Request::History {
            camera,
            from_ms,
            to_ms,
        } => {
            let Some(store) = host.history else {
                return error("o daemon está sem histórico (o banco não abriu)");
            };
            if from_ms > to_ms {
                return error("intervalo invertido (from_ms > to_ms)");
            }
            let names: Vec<String> = match camera {
                Some(n) => vec![n.clone()],
                None => host.engine.names.clone(),
            };
            let mut segments = Vec::new();
            for name in &names {
                match store.segments_between(name, *from_ms, *to_ms) {
                    Ok(rows) => segments.extend(rows.into_iter().map(|s| SegmentInfo {
                        file: relative(host.recordings, &s.path),
                        id: s.id,
                        camera: s.camera,
                        ts_start: s.ts_start,
                        ts_end: s.ts_end,
                        bytes: s.bytes,
                        has_motion: s.has_motion,
                        protected: s.protected,
                        mode: s.mode,
                    })),
                    Err(e) => return error(format!("histórico: {e}")),
                }
            }
            segments.sort_by_key(|s| s.ts_start);
            let events = match store.events_between(camera.as_deref(), *from_ms, *to_ms) {
                Ok(rows) => rows
                    .into_iter()
                    .map(|e| HistoryEvent {
                        id: e.id,
                        camera: e.camera,
                        ts: e.ts,
                        kind: e.kind,
                        label: e.label,
                        segment_id: e.segment_id,
                    })
                    .collect::<Vec<_>>(),
                Err(e) => return error(format!("histórico: {e}")),
            };
            let mut events = events;
            let truncated = segments.len() > HISTORY_LIMIT || events.len() > HISTORY_LIMIT;
            segments.truncate(HISTORY_LIMIT);
            events.truncate(HISTORY_LIMIT);
            Response::History {
                segments,
                events,
                truncated,
            }
        }
        Request::ExportClip {
            camera,
            from_ms,
            to_ms,
        } => {
            let (Some(store), Some(dir)) = (host.history, host.recordings) else {
                return error("o daemon está sem histórico (o banco não abriu)");
            };
            if from_ms >= to_ms {
                return error("intervalo vazio ou invertido");
            }
            if to_ms - from_ms > MAX_CLIP_MS {
                return error("o clipe passa de 30 minutos; peça um intervalo menor");
            }
            let rows = match store.segments_between(camera, *from_ms, *to_ms) {
                Ok(r) => r,
                Err(e) => return error(format!("histórico: {e}")),
            };
            let sources: Vec<ClipSource> = rows
                .iter()
                .filter(|s| std::path::Path::new(&s.path).exists())
                .map(|s| ClipSource {
                    path: s.path.clone().into(),
                    start_ms: s.ts_start,
                })
                .collect();
            let stamp = chrono::DateTime::from_timestamp_millis(*from_ms)
                .map(|t| t.format("%Y%m%d-%H%M%S").to_string())
                .unwrap_or_else(|| from_ms.to_string());
            let name = format!(
                "exports/{}-{stamp}.mp4",
                crate::infrastructure::recording_paths::safe_filename(camera)
            );
            match export_clip(&sources, *from_ms, *to_ms, &dir.join(&name)) {
                Ok(info) => Response::Exported {
                    file: name,
                    bytes: info.bytes,
                },
                Err(e) => error(e),
            }
        }
        Request::SetProtected {
            segment_id,
            protected,
        } => {
            let Some(store) = host.history else {
                return error("o daemon está sem histórico (o banco não abriu)");
            };
            match store.set_protected(*segment_id, *protected) {
                Ok(true) => Response::Ok,
                Ok(false) => error(format!("o segmento {segment_id} não existe mais")),
                Err(e) => error(format!("histórico: {e}")),
            }
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
                history: None,
                recordings: None,
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
    fn history_returns_segments_and_events_of_the_asked_camera_only() {
        let mut e = engine(2);
        let mut z = ZonesFile::default();
        let store = Store::open_in_memory().unwrap();
        store
            .segment_opened("cam1", "/d/a.mkv", 1_000, "motion")
            .unwrap();
        store.segment_closed("/d/a.mkv", 9_000, 4096).unwrap();
        store
            .segment_opened("cam0", "/d/b.mkv", 2_000, "manual")
            .unwrap();
        store
            .insert_event("cam1", 1_500, "motion", "", None)
            .unwrap();
        let ask = |camera: Option<&str>, e: &mut Engine, z: &mut ZonesFile| {
            apply(
                &mut Host {
                    engine: e,
                    zones_file: z,
                    persist: false,
                    history: Some(&store),
                    recordings: None,
                },
                &Request::History {
                    camera: camera.map(String::from),
                    from_ms: 0,
                    to_ms: 10_000,
                },
            )
        };
        let Response::History {
            segments,
            events,
            truncated,
        } = ask(Some("cam1"), &mut e, &mut z)
        else {
            panic!("esperava History");
        };
        assert!(!truncated);
        assert_eq!(segments.len(), 1);
        assert_eq!((segments[0].bytes, segments[0].ts_end), (4096, Some(9_000)));
        assert!(segments[0].has_motion && segments[0].mode == "motion");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].segment_id, Some(segments[0].id));

        let Response::History { segments, .. } = ask(None, &mut e, &mut z) else {
            panic!()
        };
        assert_eq!(segments.len(), 2, "sem nome: todas as câmeras");
        assert!(segments[0].ts_start <= segments[1].ts_start, "em ordem");
    }

    #[test]
    fn history_without_a_database_or_with_a_bad_interval_is_an_error() {
        let mut e = engine(1);
        let mut z = ZonesFile::default();
        let req = Request::History {
            camera: None,
            from_ms: 0,
            to_ms: 1,
        };
        let Response::Error { message } = run(&mut e, &mut z, &req) else {
            panic!()
        };
        assert!(message.contains("sem histórico"), "{message}");

        let store = Store::open_in_memory().unwrap();
        let Response::Error { message } = apply(
            &mut Host {
                engine: &mut e,
                zones_file: &mut z,
                persist: false,
                history: Some(&store),
                recordings: None,
            },
            &Request::History {
                camera: None,
                from_ms: 5,
                to_ms: 1,
            },
        ) else {
            panic!()
        };
        assert!(message.contains("invertido"), "{message}");
    }

    #[test]
    fn export_writes_a_clip_and_history_names_files_relative_to_the_recordings_dir() {
        use gstreamer as gst;
        use gstreamer::prelude::*;
        let _ = gst::init();
        let dir = std::env::temp_dir().join(format!("rrv-ipc-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let seg = dir.join("cam1-000.mkv");
        let p = gst::parse::launch(&format!(
            "videotestsrc num-buffers=60 ! video/x-raw,format=I420,width=160,height=120,framerate=30/1 \
             ! x264enc tune=zerolatency key-int-max=10 ! h264parse ! matroskamux ! filesink location={}",
            seg.display()
        ))
        .unwrap();
        p.set_state(gst::State::Playing).unwrap();
        p.bus()
            .unwrap()
            .timed_pop_filtered(
                gst::ClockTime::from_seconds(20),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
            .unwrap();
        p.set_state(gst::State::Null).unwrap();

        let mut e = engine(1);
        let store = Store::open_in_memory().unwrap();
        let path = seg.to_string_lossy().to_string();
        store
            .segment_opened("cam0", &path, 100_000, "manual")
            .unwrap();
        store.segment_closed(&path, 102_000, 1).unwrap();
        let mut z = ZonesFile::default();
        let mut host = Host {
            engine: &mut e,
            zones_file: &mut z,
            persist: false,
            history: Some(&store),
            recordings: Some(&dir),
        };

        let Response::History { segments, .. } = apply(
            &mut host,
            &Request::History {
                camera: Some("cam0".into()),
                from_ms: 0,
                to_ms: 200_000,
            },
        ) else {
            panic!()
        };
        assert_eq!(segments[0].file, "cam1-000.mkv");

        let Response::Exported { file, bytes } = apply(
            &mut host,
            &Request::ExportClip {
                camera: "cam0".into(),
                from_ms: 100_500,
                to_ms: 101_500,
            },
        ) else {
            panic!("esperava Exported")
        };
        assert!(
            file.starts_with("exports/cam0-") && file.ends_with(".mp4"),
            "{file}"
        );
        assert!(bytes > 0 && dir.join(&file).exists());

        // pedidos ruins
        for (from_ms, to_ms, expect) in [(5, 5, "vazio"), (0, 31 * 60 * 1000, "30 minutos")] {
            let Response::Error { message } = apply(
                &mut host,
                &Request::ExportClip {
                    camera: "cam0".into(),
                    from_ms,
                    to_ms,
                },
            ) else {
                panic!()
            };
            assert!(message.contains(expect), "{message}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn protecting_a_segment_goes_through_the_store_and_unknown_ids_are_an_error() {
        let mut e = engine(1);
        let mut z = ZonesFile::default();
        let store = Store::open_in_memory().unwrap();
        let id = store
            .segment_opened("cam0", "/d/a.mkv", 1, "manual")
            .unwrap();
        store.segment_closed("/d/a.mkv", 2, 10).unwrap();
        let mut host = Host {
            engine: &mut e,
            zones_file: &mut z,
            persist: false,
            history: Some(&store),
            recordings: None,
        };
        let ask = |host: &mut Host<'_>, segment_id, protected| {
            apply(
                host,
                &Request::SetProtected {
                    segment_id,
                    protected,
                },
            )
        };
        assert_eq!(ask(&mut host, id, true), Response::Ok);
        assert!(
            store.oldest_deletable(10, None).unwrap().is_empty(),
            "protegido não é candidato"
        );
        assert_eq!(ask(&mut host, id, false), Response::Ok);
        assert_eq!(store.oldest_deletable(10, None).unwrap().len(), 1);
        let Response::Error { message } = ask(&mut host, 999, true) else {
            panic!()
        };
        assert!(message.contains("não existe"), "{message}");
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
