//! Áudio por câmera: pipelines só de áudio, abertos sob demanda (a câmera ouvida), e o
//! estado do medidor de nível (VU).
//!
//! Cada câmera ouvida tem o seu pipeline (`build_audio_pipeline_for_url`: RTSP ou HLS/HTTP)
//! com `... ! volume name=audio_volume ! level ! autoaudiosink`; o volume e o mudo são
//! propriedades do `volume` ajustadas em `ui::update`, e o elemento `level` posta o RMS em
//! dB no bus, que `poll_level_bus` lê para o [`AudioLevelState`] (sem tocar no GStreamer de
//! outra thread). As URLs passam por `quote_launch_value` e saem mascaradas nos logs.

use std::sync::atomic::{AtomicU64, Ordering};

use gst::prelude::*;
use gstreamer as gst;

use log::warn;

use crate::domain::redact::mask_credentials;
use crate::infrastructure::launch::quote_launch_value;

/// Shared state for the VU meter. The `level` GStreamer element posts
/// RMS dB values on the bus every 100ms; a bus-watch extracts them and
/// stores them here so the GTK tick loop can read them without touching
/// GStreamer from the wrong thread.
///
/// The value is stored as `f64` bits in an `AtomicU64` (lock-free).
/// The VU meter normalises the dB range `[-60, 0]` → `[0.0, 1.0]`.
pub struct AudioLevelState {
    rms_db: AtomicU64,
}

impl AudioLevelState {
    pub fn new() -> Self {
        Self {
            rms_db: AtomicU64::new(f64::to_bits(-60.0)),
        }
    }

    pub fn set_rms_db(&self, db: f64) {
        self.rms_db.store(db.to_bits(), Ordering::Relaxed);
    }

    /// Returns the current RMS level normalised to `[0.0, 1.0]`.
    /// dB range `[-60, 0]` maps to `[0.0, 1.0]`.
    pub fn level(&self) -> f64 {
        let db = f64::from_bits(self.rms_db.load(Ordering::Relaxed));
        ((db + 60.0) / 60.0).clamp(0.0, 1.0)
    }

    /// Returns the raw RMS dB value (e.g. `-12.5`).
    pub fn rms_db(&self) -> f64 {
        f64::from_bits(self.rms_db.load(Ordering::Relaxed))
    }
}

impl Default for AudioLevelState {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the RMS dB value from a `level` element message structure and store
/// it in `state`. Returns `true` if a value was found.
fn apply_level_structure(s: &gst::StructureRef, state: &AudioLevelState) -> bool {
    if s.name() != "level" {
        return false;
    }
    let Ok(rms_val) = s.get::<gst::glib::Value>("rms") else {
        return false;
    };
    let Ok(arr) = rms_val.get::<gst::glib::ValueArray>() else {
        return false;
    };
    for val in arr.iter() {
        if let Ok(rms_db) = val.get::<f64>() {
            state.set_rms_db(rms_db);
            return true;
        }
    }
    false
}

/// Drain the `level` element messages off an audio pipeline's bus and update
/// the shared `AudioLevelState`. Non-blocking; call once per frame tick.
///
/// The app runs no GLib main loop, so a `bus.add_watch` would never fire —
/// the video path polls the bus the same way (`GStreamerBridge::poll_bus`).
pub fn poll_level_bus(pipeline: &gst::Pipeline, level_state: &AudioLevelState) {
    let Some(bus) = pipeline.bus() else { return };
    // `level` posts every 100 ms; a small cap keeps a backlog from stalling
    // the tick while still draining faster than messages arrive.
    for _ in 0..8 {
        let Some(msg) = bus.pop_filtered(&[gst::MessageType::Element]) else {
            break;
        };
        if let gst::MessageView::Element(e) = msg.view()
            && let Some(s) = e.structure()
        {
            apply_level_structure(s, level_state);
        }
    }
}

/// Build a standalone audio-only GStreamer pipeline for an RTSP URL.
///
/// The single-camera video pipeline rejects audio at the source
/// (`select-stream`) and decodes only video, so there is no audio branch to
/// tap. Mirroring the multi-camera grid, we run a second pipeline dedicated to
/// audio. It contains a `volume name=audio_volume` element so the existing
/// `AudioController` (mute toggle, volume) drives it unchanged.
///
/// Returns `None` on parse/downcast failure. The caller owns the pipeline and
/// must set it to `Playing` and tear it down (`Null`) on shutdown.
pub fn build_audio_pipeline(url: &str, volume: f32) -> Option<gst::Pipeline> {
    let vol = volume.clamp(0.0, 1.0);
    let pipeline_str = format!(
        "rtspsrc name=audio_src location={} latency=100 protocols=tcp \
         timeout=5000000000 udp-reconnect=true ! queue ! \
         decodebin ! audioconvert ! audioresample ! \
         volume name=audio_volume volume={vol:.3} ! \
         level name=audio_level interval=100000000 ! autoaudiosink",
        quote_launch_value(url)
    );
    let pipeline = match gst::parse::launch(&pipeline_str) {
        Ok(el) => el.downcast::<gst::Pipeline>().ok()?,
        Err(e) => {
            warn!(
                "Audio pipeline parse error for {}: {}",
                mask_credentials(url),
                mask_credentials(&e.to_string())
            );
            return None;
        }
    };
    // Accept only audio streams so decodebin sees audio/x-rtp, not video.
    if let Some(src) = pipeline.by_name("audio_src") {
        src.connect("select-stream", false, |args| {
            let caps = args[2].get::<gst::Caps>().ok()?;
            let is_audio = caps
                .structure(0)
                .and_then(|s| s.get::<&str>("media").ok())
                .map(|m| m == "audio")
                .unwrap_or(false);
            Some(is_audio.to_value())
        });
    }
    Some(pipeline)
}

/// Build a standalone audio-only pipeline for any URL scheme.
///
/// Auto-detects the source type:
/// - RTSP (`rtsp://` / `rtsps://`) → uses `rtspsrc` (optimised path).
/// - HTTP/HLS (`http://` / `https://`) → uses `uridecodebin` with a
///   `pad-added` handler that routes only audio pads to the conversion
///   chain.
///
/// The volume element is named `audio_volume` (matching the constant
/// `AUDIO_VOLUME_NAME`). The caller owns the pipeline and must set it to `Playing`
/// and tear it down (`Null`) on shutdown.
pub fn build_audio_pipeline_for_url(url: &str, volume: f32) -> Option<gst::Pipeline> {
    let is_http = url.starts_with("http://") || url.starts_with("https://");
    if is_http {
        build_http_audio_pipeline(url, volume)
    } else {
        build_audio_pipeline(url, volume)
    }
}

/// Build an audio pipeline for HTTP/HTTPS (HLS) URLs using `uridecodebin`.
fn build_http_audio_pipeline(url: &str, volume: f32) -> Option<gst::Pipeline> {
    let vol = volume.clamp(0.0, 1.0);
    // uridecodebin is separated from the audio chain by a bare space so its
    // dynamic src pads remain unlinked at parse time; the pad-added handler
    // below routes audio pads to audioconvert.
    let pipeline_str = format!(
        "uridecodebin name=audio_uri uri={} \
         audioconvert name=audio_convert ! audioresample \
         ! volume name=audio_volume volume={vol:.3} ! \
         level name=audio_level interval=100000000 ! autoaudiosink",
        quote_launch_value(url)
    );
    let pipeline = match gst::parse::launch(&pipeline_str) {
        Ok(el) => el.downcast::<gst::Pipeline>().ok()?,
        Err(e) => {
            warn!(
                "HTTP audio pipeline parse error for {}: {}",
                mask_credentials(url),
                mask_credentials(&e.to_string())
            );
            return None;
        }
    };
    // Route audio pads from uridecodebin to audioconvert; ignore non-audio
    // pads (video, subtitles, etc.).
    let pipeline_weak = pipeline.downgrade();
    if let Some(dec) = pipeline.by_name("audio_uri") {
        dec.connect_pad_added(move |_dec, pad| {
            let is_audio = pad
                .current_caps()
                .or_else(|| pad.allowed_caps())
                .and_then(|c| c.structure(0).map(|s| s.name().starts_with("audio/")))
                .unwrap_or(false);
            if !is_audio {
                return;
            }
            let Some(pl) = pipeline_weak.upgrade() else {
                return;
            };
            let Some(conv) = pl.by_name("audio_convert") else {
                return;
            };
            let Some(sink) = conv.static_pad("sink") else {
                return;
            };
            if let Err(e) = pad.link(&sink) {
                warn!("HTTP audio pad link failed: {:?}", e);
            }
        });
    }
    Some(pipeline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_pipelines_survive_hostile_urls() {
        if gst::init().is_err() {
            return;
        }
        // `"`, `&`, `!` and spaces would break an unquoted description.
        assert!(build_audio_pipeline("rtsp://u:p@h/a b\"c&d!e", 0.5).is_some());
        assert!(build_http_audio_pipeline("http://h/p?a=1&b=2 !x", 0.5).is_some());
    }
}
