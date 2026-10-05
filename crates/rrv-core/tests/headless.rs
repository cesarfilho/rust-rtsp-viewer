//! O motor sem janela (`Engine::new` + `Engine::step` + `Engine::shutdown`),
//! com uma câmera HLS **ao vivo** simulada só com GStreamer. É o que o daemon
//! executa (ADR 0010).
//!
//! A fonte tem de ser em tempo real: um arquivo é decodificado mais rápido que
//! o relógio e entrega quadros em rajadas, o que torna a detecção de movimento
//! e a gravação não determinísticas (não é como uma câmera se comporta).
//!
//! Precisam de `x264enc` e `hlssink2` (plugins-ugly e plugins-bad).

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use rrv_core::config::{CameraConfig, LogsConfigFile};
use rrv_core::domain::camera_status::CameraStatus;
use rrv_core::domain::motion::MotionConfig;
use rrv_core::domain::notify::NotifyConfig;
use rrv_core::domain::recording::RecordingConfig;
use rrv_core::domain::timeline::EventType;
use rrv_core::engine::{Engine, EngineEvent, EngineSettings, TICK_MS};
use rrv_core::infrastructure::zone_state::ZonesFile;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        let d = std::env::temp_dir().join(format!("rrv-hl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
    fn path(&self, n: &str) -> PathBuf {
        self.0.join(n)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Uma câmera HLS ao vivo: `videotestsrc` em tempo real → `hlssink2` gravando
/// segmentos de 1 s, servidos por um HTTP mínimo em 127.0.0.1.
struct LiveCamera {
    pipeline: gst::Pipeline,
    src: gst::Element,
    stop: Arc<AtomicBool>,
    port: u16,
}

impl LiveCamera {
    fn start(dir: &Path) -> Self {
        gst::init().unwrap();
        let hls = dir.join("hls");
        std::fs::create_dir_all(&hls).unwrap();
        let desc = format!(
            "videotestsrc name=src is-live=true pattern=ball \
             ! video/x-raw,width=320,height=180,framerate=30/1 \
             ! videoconvert ! x264enc tune=zerolatency key-int-max=30 bitrate=400 \
             ! h264parse \
             ! hlssink2 location=\"{d}/seg%05d.ts\" playlist-location=\"{d}/live.m3u8\" \
               target-duration=1 max-files=10 playlist-length=6",
            d = hls.display()
        );
        let pipeline = gst::parse::launch(&desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        let src = pipeline.by_name("src").unwrap();
        pipeline.set_state(gst::State::Playing).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((conn, _)) => {
                        let hls = hls.clone();
                        std::thread::spawn(move || serve(conn, &hls));
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        });
        let cam = Self {
            pipeline,
            src,
            stop,
            port,
        };
        // A câmera só "existe" quando já publicou segmentos suficientes.
        let playlist = dir.join("hls/live.m3u8");
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let ready = std::fs::read_to_string(&playlist)
                .map(|t| t.matches(".ts").count() >= 3)
                .unwrap_or(false);
            if ready {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        cam
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/live.m3u8", self.port)
    }

    /// A bola se mexe, ou a imagem fica parada.
    fn set_moving(&self, moving: bool) {
        self.src
            .set_property_from_str("pattern", if moving { "ball" } else { "black" });
    }
}

impl Drop for LiveCamera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// HTTP/1.0 mínimo: serve arquivos de `root`.
fn serve(mut conn: TcpStream, root: &Path) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buf = [0u8; 2048];
    let n = conn.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("/")
        .trim_start_matches('/')
        .split('?')
        .next()
        .unwrap_or("")
        .to_string();
    if path.contains("..") {
        let _ = conn.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        return;
    }
    match std::fs::read(root.join(&path)) {
        Ok(body) => {
            let ctype = if path.ends_with(".m3u8") {
                "application/vnd.apple.mpegurl"
            } else {
                "video/mp2t"
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = conn.write_all(head.as_bytes());
            let _ = conn.write_all(&body);
        }
        Err(_) => {
            let _ = conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        }
    }
}

fn engine(url: &str, rec_dir: &Path, post_roll: u32) -> Engine {
    let cam: CameraConfig = toml::from_str(&format!("url = {url:?}\nname = \"cam\"")).unwrap();
    let recording = RecordingConfig {
        dir: rec_dir.to_path_buf(),
        on_motion: true,
        motion_post_roll_secs: post_roll,
        ..RecordingConfig::default()
    };
    let logs = LogsConfigFile {
        dir: Some(rec_dir.join("logs")),
        ..LogsConfigFile::default()
    };
    Engine::new(EngineSettings {
        cameras: &[cam],
        recording: &recording,
        // Mais sensível que o padrão: a bola do `videotestsrc`, comprimida a
        // 400 kbps, mexe pouco do quadro entre duas amostras de 500 ms.
        motion: MotionConfig {
            sample_stride: 2,
            contour_area: 0.002,
            threshold: 20,
            ..MotionConfig::default()
        },
        notify: NotifyConfig {
            enabled: false,
            ..NotifyConfig::default()
        },
        logs: &logs,
        pause_hidden: true,
        stagger: Duration::from_millis(100),
        zones: &ZonesFile::default(),
    })
}

/// Run `step` at the real tick until `done` says so or `secs` pass.
fn run(
    e: &mut Engine,
    secs: u64,
    events: &mut Vec<EngineEvent>,
    mut done: impl FnMut(&Engine, &[EngineEvent]) -> bool,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut tick = 0u64;
    while Instant::now() < deadline {
        events.extend(e.step(tick));
        tick += 1;
        if done(e, events) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(TICK_MS));
    }
    false
}

fn mkv_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    v.retain(|p| p.extension().is_some_and(|x| x == "mkv"));
    v.sort();
    v
}

fn is_playable(path: &Path) -> bool {
    let desc = format!(
        "filesrc location=\"{}\" ! decodebin ! fakesink sync=false",
        path.display()
    );
    let p = gst::parse::launch(&desc).unwrap();
    if p.set_state(gst::State::Playing).is_err() {
        return false;
    }
    let ok = p
        .bus()
        .and_then(|b| {
            b.timed_pop_filtered(
                gst::ClockTime::from_seconds(10),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
        })
        .is_some_and(|m| matches!(m.view(), gst::MessageView::Eos(_)));
    let _ = p.set_state(gst::State::Null);
    ok
}

/// Estado do pipeline do motor, para mensagens de falha.
fn describe(e: &Engine) -> String {
    let pl = e.bridges[0].lock().unwrap().pipeline().cloned();
    let Some(pl) = pl else {
        return "sem pipeline".into();
    };
    let (r, cur, pend) = pl.state(gst::ClockTime::from_mseconds(50));
    let mut levels = Vec::new();
    for name in ["postdec_queue", "display_queue", "detect_queue"] {
        if let Some(q) = pl.by_name(name) {
            levels.push(format!(
                "{name}={}",
                q.property::<u32>("current-level-buffers")
            ));
        }
    }
    let mut sinks = Vec::new();
    let mut not_playing = Vec::new();
    let mut it = pl.iterate_recurse();
    loop {
        match it.next() {
            Ok(Some(el)) => {
                {
                    let (_, c, p) = el.state(gst::ClockTime::from_mseconds(10));
                    if c != gst::State::Playing || p != gst::State::VoidPending {
                        let f = el
                            .factory()
                            .map(|f| f.name().to_string())
                            .unwrap_or_default();
                        not_playing.push(format!("{}[{f}]:{c:?}→{p:?}", el.name()));
                    }
                }
                if el.factory().is_some_and(|f| f.klass().contains("Sink")) {
                    let (_, c, p) = el.state(gst::ClockTime::from_mseconds(10));
                    let asyn = if el.has_property("async") {
                        format!(" async={}", el.property::<bool>("async"))
                    } else {
                        String::new()
                    };
                    sinks.push(format!("{}:{c:?}→{p:?}{asyn}", el.name()));
                }
            }
            Ok(None) => break,
            Err(gst::IteratorError::Resync) => {
                sinks.clear();
                not_playing.clear();
                it.resync();
            }
            Err(_) => break,
        }
    }
    format!(
        "pipeline {r:?} {cur:?}→{pend:?}; filas: {}; sinks: {}; NÃO em PLAYING: {}",
        levels.join(" "),
        sinks.join(", "),
        not_playing.join(", ")
    )
}

fn kinds(events: &[EngineEvent]) -> Vec<EventType> {
    events.iter().map(|e| e.kind).collect()
}

/// Sem janela, o motor sobe a câmera sozinho, detecta o movimento, grava e para
/// pelo pós-roll, deixando um arquivo completo.
#[test]
fn the_headless_engine_records_on_motion_and_stops_after_the_post_roll() {
    let tmp = TempDir::new("motion");
    let cam = LiveCamera::start(&tmp.0);
    let rec = tmp.path("rec");
    let mut e = engine(&cam.url(), &rec, 3);

    let mut events = Vec::new();
    // A bola se mexe até a gravação começar; então a cena para.
    let started = run(&mut e, 60, &mut events, |_, ev| {
        kinds(ev).contains(&EventType::RecordingStart)
    });
    assert!(
        started,
        "a gravação por movimento nunca começou: {:?}",
        kinds(&events)
    );
    cam.set_moving(false);
    let stopped = run(&mut e, 60, &mut events, |_, ev| {
        kinds(ev).contains(&EventType::RecordingStop)
    });
    e.shutdown();

    let k = kinds(&events);
    assert!(stopped, "a gravação não parou pelo pós-roll: {k:?}");
    assert!(k.contains(&EventType::Motion), "{k:?}");
    let files = mkv_files(&rec);
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(
        std::fs::metadata(&files[0]).unwrap().len() > 1024,
        "arquivo quase vazio"
    );
    assert!(is_playable(&files[0]), "{} não toca", files[0].display());
}

/// O defeito do SIGTERM: parar com uma gravação em curso tem de deixar o
/// arquivo finalizado, não vazio.
#[test]
fn shutdown_during_a_recording_leaves_a_playable_file() {
    let tmp = TempDir::new("shutdown");
    let cam = LiveCamera::start(&tmp.0); // a bola nunca para
    let rec = tmp.path("rec");
    let mut e = engine(&cam.url(), &rec, 3600);

    let mut events = Vec::new();
    assert!(
        run(&mut e, 60, &mut events, |_, ev| kinds(ev)
            .contains(&EventType::RecordingStart)),
        "a gravação por movimento nunca começou: {:?}",
        kinds(&events)
    );
    // O vídeo tem de continuar fluindo enquanto grava. Regressão: o
    // `splitmuxsink` assíncrono congelava o tee (imagem e detecção) até o ramo
    // de gravação ser removido.
    let generation = |e: &Engine| e.bridges[0].lock().unwrap().read_frame().3;
    let before = generation(&e);
    run(&mut e, 4, &mut Vec::new(), |_, _| false);
    let advanced = generation(&e) - before;
    assert!(
        advanced >= 30,
        "o display congelou durante a gravação: só {advanced} quadros em 4 s ({})",
        describe(&e)
    );

    e.shutdown();

    assert!(
        e.status.iter().all(|s| *s != CameraStatus::Recording),
        "nada deve seguir gravando depois do shutdown: {:?}",
        e.status
    );
    let files = mkv_files(&rec);
    assert_eq!(files.len(), 1, "{files:?}");
    let size = std::fs::metadata(&files[0]).unwrap().len();
    assert!(size > 1024, "arquivo quase vazio ({size} bytes)");
    assert!(is_playable(&files[0]), "{} não toca", files[0].display());
}

#[test]
fn the_headless_engine_brings_a_camera_up_and_keeps_it_live() {
    let tmp = TempDir::new("live");
    let cam = LiveCamera::start(&tmp.0);
    cam.set_moving(false); // cena parada: nada grava
    let mut e = engine(&cam.url(), &tmp.path("rec"), 3);

    let mut events = Vec::new();
    let live = run(&mut e, 40, &mut events, |e, _| {
        e.status[0] == CameraStatus::Live
    });
    // O status "Live" sozinho não prova nada: ele só muda quando fps > 0 e nunca
    // volta a "parado". O que importa é o vídeo fluir. Regressão: o ramo de
    // detecção deixava o pipeline preso antes de PLAYING (um quadro e congela).
    let generation = |e: &Engine| e.bridges[0].lock().unwrap().read_frame().3;
    let before = generation(&e);
    run(&mut e, 4, &mut events, |_, _| false);
    let advanced = generation(&e) - before;
    let state = describe(&e);
    e.shutdown();

    assert!(live, "a câmera não ficou ao vivo");
    assert!(
        advanced >= 30,
        "o vídeo não está fluindo: só {advanced} quadros em 4 s ({state})"
    );
    assert!(
        !kinds(&events).contains(&EventType::Offline),
        "uma câmera que está conectando não é offline: {:?}",
        kinds(&events)
    );
}
