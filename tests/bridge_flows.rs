//! Fluxos que antes eram validados à mão, agora com fontes sintéticas
//! (`videotestsrc` gravado em arquivo): troca sub/main, falha na partida com
//! recuperação e reconexão sem perder a gravação.
//!
//! Rodam GStreamer de verdade, então precisam de `x264enc` (o mesmo requisito
//! dos testes de gravação em `ui::pipeline`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gstreamer as gst;
use gstreamer::prelude::*;
use rust_rtsp_viewer::config::CameraConfig;
use rust_rtsp_viewer::ui::bridge::GStreamerBridge;

/// Diretório temporário próprio do teste, removido ao sair do escopo.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rrv-it-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Grava ~2 s de `videotestsrc` em `path` (Matroska/H.264) e espera o EOS,
/// para o arquivo sair finalizado.
fn make_clip(path: &Path, width: u32, height: u32) {
    gst::init().unwrap();
    let desc = format!(
        "videotestsrc num-buffers=60 ! video/x-raw,width={width},height={height},framerate=30/1 \
         ! videoconvert ! x264enc tune=zerolatency key-int-max=15 ! h264parse \
         ! matroskamux ! filesink location=\"{}\"",
        path.display()
    );
    let pipeline = gst::parse_launch(&desc).unwrap();
    pipeline.set_state(gst::State::Playing).unwrap();
    let msg = pipeline.bus().unwrap().timed_pop_filtered(
        gst::ClockTime::from_seconds(20),
        &[gst::MessageType::Eos, gst::MessageType::Error],
    );
    pipeline.set_state(gst::State::Null).unwrap();
    assert!(
        matches!(
            msg.as_ref().map(|m| m.view()),
            Some(gst::MessageView::Eos(_))
        ),
        "could not generate test clip {}: {msg:?}",
        path.display()
    );
}

fn camera(url: &Path) -> CameraConfig {
    toml::from_str(&format!("url = {:?}", url.to_string_lossy())).unwrap()
}

/// Espera `cond` ficar verdadeira, drenando o barramento do bridge no meio.
fn wait_for(
    bridge: &GStreamerBridge,
    secs: u64,
    mut cond: impl FnMut(&GStreamerBridge) -> bool,
) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        bridge.poll_bus();
        if cond(bridge) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn frame_size(bridge: &GStreamerBridge) -> Option<(u32, u32)> {
    bridge.capture_frame().map(|(_, w, h)| (w, h))
}

fn has_error(bridge: &GStreamerBridge) -> bool {
    bridge
        .error_message
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
}

/// A troca sub ↔ main reconstrói o pipeline no mesmo bridge: o tamanho do
/// quadro tem de acompanhar a nova fonte.
#[test]
fn switching_stream_rebuilds_with_the_new_resolution() {
    let tmp = TempDir::new("switch");
    let sub = tmp.path("sub.mkv");
    let main = tmp.path("main.mkv");
    make_clip(&sub, 320, 180);
    make_clip(&main, 640, 360);

    let mut bridge = GStreamerBridge::new(320, 180).unwrap();

    bridge.start_from_config(&camera(&sub)).unwrap();
    assert!(
        wait_for(&bridge, 10, |b| frame_size(b) == Some((320, 180))),
        "sub-stream frames never arrived"
    );

    // `start_*` para o pipeline anterior, então reaproveitar o bridge é a troca.
    bridge.start_from_config(&camera(&main)).unwrap();
    assert!(
        wait_for(&bridge, 10, |b| frame_size(b) == Some((640, 360))),
        "frames did not follow the switch to the main stream"
    );
    assert!(!has_error(&bridge), "switch left a stale error behind");
    bridge.stop();
}

/// Fonte inexistente: o erro aparece no bridge, e uma nova tentativa com a
/// fonte disponível se recupera sem resíduo do erro anterior.
#[test]
fn a_failed_start_recovers_once_the_source_exists() {
    let tmp = TempDir::new("recover");
    let clip = tmp.path("late.mkv");

    let mut bridge = GStreamerBridge::new(320, 180).unwrap();
    // `start_*` pode falhar de imediato ou só reportar pelo barramento.
    let started = bridge.start_from_config(&camera(&clip));
    if started.is_ok() {
        assert!(
            wait_for(&bridge, 10, has_error),
            "a missing source never reported an error"
        );
    }

    make_clip(&clip, 320, 180);
    bridge.stop();
    assert!(!has_error(&bridge), "stop() must clear the stale error");

    bridge.start_from_config(&camera(&clip)).unwrap();
    assert!(
        wait_for(&bridge, 10, |b| frame_size(b).is_some()),
        "camera did not recover after the source appeared"
    );
    assert!(bridge.is_live());
    assert!(!has_error(&bridge));
    bridge.stop();
}

/// A reconexão (`stop` + `start_from_config`) com gravação em curso fecha o
/// segmento atual; religar a gravação abre outro, e os dois tocam até o EOS.
#[test]
fn reconnecting_keeps_recording_into_a_fresh_segment() {
    let tmp = TempDir::new("reconnect");
    let clip = tmp.path("src.mkv");
    make_clip(&clip, 320, 180);
    let rec_dir = tmp.path("rec");

    let mut bridge = GStreamerBridge::new(320, 180).unwrap();
    bridge.recording_config.dir = rec_dir.clone();
    bridge.start_from_config(&camera(&clip)).unwrap();
    assert!(wait_for(&bridge, 10, |b| frame_size(b).is_some()));

    bridge.start_recording().unwrap();
    std::thread::sleep(Duration::from_millis(1500));

    // O que `reconnect_camera` faz: lembrar, parar, refazer, retomar.
    let was_recording = bridge.is_recording();
    assert!(was_recording);
    bridge.stop();
    assert!(!bridge.is_recording(), "stop() must finalise the recording");
    bridge.start_from_config(&camera(&clip)).unwrap();
    assert!(wait_for(&bridge, 10, |b| frame_size(b).is_some()));
    bridge.start_recording().unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    bridge.stop();

    let mut segments: Vec<_> = std::fs::read_dir(&rec_dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "mkv"))
        .collect();
    segments.sort();
    assert_eq!(segments.len(), 2, "expected 2 segments, got {segments:?}");
    for seg in &segments {
        assert!(
            is_playable(seg),
            "{} is not a finalised file",
            seg.display()
        );
    }
}

fn is_playable(path: &Path) -> bool {
    let desc = format!(
        "filesrc location=\"{}\" ! decodebin ! fakesink sync=false",
        path.display()
    );
    let pipeline = gst::parse_launch(&desc).unwrap();
    if pipeline.set_state(gst::State::Playing).is_err() {
        return false;
    }
    let msg = pipeline.bus().and_then(|bus| {
        bus.timed_pop_filtered(
            gst::ClockTime::from_seconds(10),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        )
    });
    let ok = msg
        .as_ref()
        .is_some_and(|m| matches!(m.view(), gst::MessageView::Eos(_)));
    if !ok {
        eprintln!(
            "{}: {} bytes, bus: {msg:?}",
            path.display(),
            std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
        );
    }
    let _ = pipeline.set_state(gst::State::Null);
    ok
}
