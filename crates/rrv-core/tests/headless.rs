//! O motor sem janela (`Engine::new` + `Engine::step` + `Engine::shutdown`),
//! com uma câmera HLS **ao vivo** simulada só com GStreamer. É o que o daemon
//! executa (ADR 0010).
//!
//! A fonte tem de ser em tempo real: um arquivo é decodificado mais rápido que
//! o relógio e entrega quadros em rajadas, o que torna a detecção de movimento
//! e a gravação não determinísticas (não é como uma câmera se comporta).
//!
//! Precisam de `x264enc` e `hlssink2` (plugins-ugly e plugins-bad).

use std::path::Path;
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
use rrv_core::testing::{LiveCamera, TempDir, is_playable, mkv_files};

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
    // O decodificador de software criado pelo `decodebin` sai com os threads
    // limitados (senão usa um por núcleo e a memória cresce com as câmeras).
    let threads: Vec<i32> = {
        let pl = e.bridges[0].lock().unwrap().pipeline().cloned().unwrap();
        let mut found = Vec::new();
        let mut it = pl.iterate_recurse();
        while let Ok(Some(el)) = it.next() {
            if el.factory().is_some_and(|f| f.name().starts_with("avdec_")) {
                found.push(el.property::<i32>("max-threads"));
            }
        }
        found
    };
    e.shutdown();

    assert!(
        !threads.is_empty() && threads.iter().all(|&t| t == 2),
        "decodificadores de software sem o limite de threads: {threads:?}"
    );
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

/// Modo display-only (a janela conectada a um daemon): o vídeo flui, mas nada
/// grava, detecta ou emite evento, mesmo com movimento e `on_motion` ligado. Ao
/// voltar ao modo completo ("Usar motor local"), a gravação por movimento
/// começa e o arquivo sai tocável.
#[test]
fn display_only_shows_video_but_never_records_until_switched_back() {
    let tmp = TempDir::new("display-only");
    let cam = LiveCamera::start(&tmp.0); // a bola se mexe
    let rec = tmp.path("rec");
    let mut e = engine(&cam.url(), &rec, 3600);
    e.set_display_only(true);

    let mut events = Vec::new();
    let generation = |e: &Engine| e.bridges[0].lock().unwrap().read_frame().3;
    // espera o vídeo começar, depois mede o fluxo por 5 s
    assert!(
        run(&mut e, 40, &mut events, |e, _| generation(e) > 0),
        "o vídeo nunca começou"
    );
    let before = generation(&e);
    run(&mut e, 5, &mut events, |_, _| false);
    let advanced = generation(&e) - before;
    let state = describe(&e);

    assert!(
        advanced >= 30,
        "o vídeo não está fluindo em display-only: {advanced} quadros ({state})"
    );
    assert!(
        e.bridges[0]
            .lock()
            .unwrap()
            .capture_detect_frame()
            .is_none(),
        "display-only não monta o ramo de detecção"
    );
    assert!(
        events.is_empty(),
        "display-only não emite eventos: {:?}",
        kinds(&events)
    );
    assert!(mkv_files(&rec).is_empty(), "display-only não grava");
    assert!(e.toggle_recording(0).is_err());

    // "Usar motor local": agora o motor é dono da gravação
    e.set_display_only(false);
    let started = run(&mut e, 60, &mut events, |_, ev| {
        kinds(ev).contains(&EventType::RecordingStart)
    });
    run(&mut e, 3, &mut events, |_, _| false);
    e.shutdown();

    assert!(
        started,
        "a gravação por movimento não começou ao sair do display-only"
    );
    let files = mkv_files(&rec);
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(is_playable(&files[0]), "{} não toca", files[0].display());
}

/// Ferramenta de diagnóstico manual (ignorada): `RRV_PROBE_URL=rtsp://… cargo test -p rrv-core
/// --test headless probe_url -- --ignored --nocapture`. Sobe o motor numa URL real e imprime, a cada
/// segundo, o contador de quadros e o estado do pipeline (filas, elementos que não estão em PLAYING).
/// A URL pode levar `${SEGREDO}`; nada disto imprime a URL.
#[test]
#[ignore]
fn probe_url() {
    let Some(raw) = std::env::var_os("RRV_PROBE_URL") else {
        eprintln!("defina RRV_PROBE_URL");
        return;
    };
    let url = rrv_core::secrets::expand(&raw.to_string_lossy(), &rrv_core::secrets::lookup)
        .expect("segredo ausente");
    let tmp = TempDir::new("probe");
    let mut e = engine(&url, &tmp.path("rec"), 3600);
    e.set_display_only(true);
    let generation = |e: &Engine| e.bridges[0].lock().unwrap().read_frame().3;
    let secs: u64 = std::env::var("RRV_PROBE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20);
    let start = Instant::now();
    let mut tick = 0u64;
    let mut last = 0u64;
    while start.elapsed() < Duration::from_secs(secs) {
        e.step(tick);
        if tick.is_multiple_of(10) {
            let g = generation(&e);
            eprintln!(
                "t={:>4.1}s quadros(+{}) status={:?}",
                tick as f64 * 0.1,
                g - last,
                e.status[0]
            );
            last = g;
        }
        if tick == 100 {
            eprintln!("{}", describe(&e));
        }
        tick += 1;
        std::thread::sleep(Duration::from_millis(TICK_MS));
    }
    e.shutdown();
}
