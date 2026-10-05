//! Audio controller — controls the audio branch of
//! the pipeline (item 11). The `audio_volume`
//! element is a `volume` element in the audio branch
//! of the pipeline tee; the controller toggles its
//! `volume` property to mute / unmute.
//!
//! Pipeline shape (see `pipeline::Segment::AudioBranch`):
//!   t_audio. ! audioconvert ! audioresample !
//!             volume name=audio_volume ! level !
//!             autoaudiosink
//!
//! Element is `volume` (not `liveadder` or a
//! custom audio filter) because:
//! - `volume` is the canonical GStreamer way to
//!   mute an audio stream — it scales the samples
//!   by a factor in `[0, 10]` (we only ever set
//!   values in `[0, 1]`).
//! - It's part of `gst-plugins-base` which is
//!   installed on every Ubuntu machine that has
//!   GTK3 + GStreamer.
//! - It exposes a single `volume` property that
//!   can be changed at runtime via
//!   `set_property` — no state-machine dance
//!   required, no event filter to install.
//!
//! A future item could swap `volume` for a
//! `audiocheblimit` / `audioecho` chain if the
//! user needs audio FX; for now the simple
//! approach gets us a working feature.

#![allow(dead_code)]

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use gstreamer as gst;
use gst::prelude::*;

use log::{info, warn};

use crate::domain::redact::mask_credentials;
use crate::infrastructure::launch::quote_launch_value;
use crate::domain::audio::{format_volume, volume_to_x1000, AudioConfig, AudioState};

/// Element name. The `AudioController` looks this element up by name in
/// whichever pipeline it is given — the dedicated audio-only pipeline built
/// by `build_audio_pipeline` (single-camera mode) or, historically, an inline
/// audio branch.
const AUDIO_VOLUME_NAME: &str = "audio_volume";

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
            && let Some(s) = e.structure() {
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
    let pipeline = match gst::parse_launch(&pipeline_str) {
        Ok(el) => el.downcast::<gst::Pipeline>().ok()?,
        Err(e) => {
            warn!("Audio pipeline parse error for {}: {}", mask_credentials(url), mask_credentials(&e.to_string()));
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
    let pipeline = match gst::parse_launch(&pipeline_str) {
        Ok(el) => el.downcast::<gst::Pipeline>().ok()?,
        Err(e) => {
            warn!("HTTP audio pipeline parse error for {}: {}", mask_credentials(url), mask_credentials(&e.to_string()));
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
            let Some(pl) = pipeline_weak.upgrade() else { return };
            let Some(conv) = pl.by_name("audio_convert") else { return };
            let Some(sink) = conv.static_pad("sink") else { return };
            if let Err(e) = pad.link(&sink) {
                warn!("HTTP audio pad link failed: {:?}", e);
            }
        });
    }
    Some(pipeline)
}

/// Owns the audio state and the pipeline
/// handle. Constructed once per `App` lifetime;
/// the pipeline is shared via `gst::Pipeline`
/// (clones share the underlying handle).
pub struct AudioController {
    pipeline: gst::Pipeline,
    config: AudioConfig,
    state: RefCell<AudioState>,
}

impl AudioController {
    /// Create a new `AudioController` from a
    /// pipeline handle and a config. Does NOT
    /// touch the pipeline — the `audio_volume`
    /// element is found lazily on the first
    /// `toggle_mute()` / `apply()` call (the
    /// pipeline may not be fully constructed at
    /// `App::new` time).
    pub fn new(pipeline: gst::Pipeline, config: AudioConfig) -> Self {
        let initial_state = if config.enabled {
            if config.volume == 0.0 {
                AudioState::Muted
            } else {
                AudioState::Live { volume_x1000: volume_to_x1000(config.volume) }
            }
        } else {
            AudioState::Muted
        };
        Self {
            pipeline,
            config,
            state: RefCell::new(initial_state),
        }
    }

    /// Read-only snapshot of the current state.
    pub fn state(&self) -> AudioState {
        *self.state.borrow()
    }

    /// True if the audio branch is currently
    /// outputting sound (volume > 0).
    pub fn is_audible(&self) -> bool {
        self.state().is_audible()
    }

    /// Push the current `AudioState` into the
    /// pipeline's `audio_volume` element. Called
    /// by `toggle_mute` and `set_volume` after a
    /// state transition. Silently no-ops when
    /// the element isn't in the pipeline yet
    /// (very early during startup, or when the
    /// pipeline was rebuilt without the audio
    /// branch).
    fn apply(&self, target: AudioState) {
        let volume = target.volume_f32();
        let Some(elem) = self.pipeline.by_name(AUDIO_VOLUME_NAME) else {
            warn!(
                "audio_volume element not in pipeline yet — \
                 cannot apply volume {volume:.2} (deferring)"
            );
            return;
        };
        // `volume` is a `gst::Element` (it's a
        // `gst::Bin` subclass in the audio bin
        // case, but here it's the leaf
        // `volume` element). `gst::ObjectExt`
        // is the trait that exposes
        // `set_property`. We do *not* set the
        // element state here — the pipeline
        // is already PLAYING (or transitioning
        // to it), so the volume change takes
        // effect on the next audio buffer.
        // GStreamer `volume` expects a `gdouble` (f64)
        let vol_f64 = volume as f64;
        elem.set_property("volume", vol_f64);
    }

    /// Toggle between `Muted` and `Live` states.
    /// The convenience entry point for the `m`
    /// keymap action. Returns the new state.
    pub fn toggle_mute(&self) -> AudioState {
        let mut state = self.state.borrow_mut();
        let (new_state, _applied_volume) = state.toggle();
        *state = new_state;
        drop(state);
        let target = self.state();
        self.apply(target);
        let vol_label = format_volume(target.volume_x1000());
        let label = if target.is_audible() {
            format!("▶ Audio unmuted ({vol_label})")
        } else {
            "■ Audio muted".to_string()
        };
        info!("{label}");
        target
    }

    /// Set the volume to a new value
    /// (clamped to `[0.0, 1.0]`). Switches
    /// state to `Live` if the new value is
    /// > 0, or to `Muted` if it's 0.
    /// > Returns the new state.
    pub fn set_volume(&self, new_volume: f32) -> AudioState {
        let v = new_volume.clamp(0.0, 1.0);
        let new_state = if v == 0.0 {
            AudioState::Muted
        } else {
            AudioState::Live { volume_x1000: volume_to_x1000(v) }
        };
        {
            let mut state = self.state.borrow_mut();
            *state = new_state;
        }
        self.apply(new_state);
        info!("Audio volume: {:.2}", new_state.volume_f32());
        new_state
    }

    /// Push the initial state into the pipeline
    /// (called by `App::new` after the pipeline
    /// has reached PLAYING — the volume element
    /// is constructed lazily so we have to wait
    /// for the bus to fire).
    pub fn apply_initial(&self) {
        let target = self.state();
        self.apply(target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::audio::AudioConfig;

    /// Build a one-element pipeline with
    /// a `volume` element named
    /// `audio_volume` (matching the name in
    /// `Segment::AudioBranch`). Returns the
    /// pipeline (caller sets it to PLAYING
    /// and tears it down).
    fn build_volume_pipeline() -> gst::Pipeline {
        gst::init().expect("gst::init");
        let pipeline = gst::Pipeline::new(None);
        let volume = gst::ElementFactory::make("volume")
            .name(AUDIO_VOLUME_NAME)
            .build()
            .expect("make volume");
        let sink = gst::ElementFactory::make("fakesink")
            .build()
            .expect("make fakesink");
        pipeline.add_many(&[&volume, &sink]).expect("add_many");
        gst::Element::link_many(&[&volume, &sink]).expect("link_many");
        pipeline
    }


    #[test]
    fn new_controller_with_enabled_config_starts_audible() {
        // Default config (enabled, volume 0.8)
        // → state is Live { 800 } which is
        // audible.
        let pipeline = build_volume_pipeline();
        let ctrl = AudioController::new(
            pipeline,
            AudioConfig { enabled: true, volume: 0.8 },
        );
        assert!(ctrl.is_audible());
        assert_eq!(
            ctrl.state(),
            crate::domain::audio::AudioState::Live { volume_x1000: 800 }
        );
    }

    #[test]
    fn new_controller_with_disabled_config_starts_muted() {
        // `enabled = false` → state is Muted
        // (the volume element gets 0.0 when
        // the pipeline reaches PLAYING).
        let pipeline = build_volume_pipeline();
        let ctrl = AudioController::new(
            pipeline,
            AudioConfig { enabled: false, volume: 0.8 },
        );
        assert!(!ctrl.is_audible());
        assert_eq!(ctrl.state(), crate::domain::audio::AudioState::Muted);
    }

    #[test]
    fn new_controller_with_zero_volume_starts_muted() {
        // `enabled = true` but `volume = 0.0`
        // → state is Muted (the volume is
        // zero, no point in `Live`).
        let pipeline = build_volume_pipeline();
        let ctrl = AudioController::new(
            pipeline,
            AudioConfig { enabled: true, volume: 0.0 },
        );
        assert!(!ctrl.is_audible());
        assert_eq!(ctrl.state(), crate::domain::audio::AudioState::Muted);
    }

    #[test]
    fn toggle_mute_from_muted_returns_live_at_080() {
        // Toggling Muted → Live { 800 } at
        // the default 0.8 volume.
        let pipeline = build_volume_pipeline();
        let ctrl = AudioController::new(
            pipeline,
            AudioConfig { enabled: false, volume: 0.8 },
        );
        let new_state = ctrl.toggle_mute();
        assert!(new_state.is_audible());
        assert_eq!(
            new_state,
            crate::domain::audio::AudioState::Live { volume_x1000: 800 }
        );
    }

    #[test]
    fn toggle_mute_from_live_returns_muted() {
        let pipeline = build_volume_pipeline();
        let ctrl = AudioController::new(
            pipeline,
            AudioConfig { enabled: true, volume: 0.6 },
        );
        let new_state = ctrl.toggle_mute();
        assert!(!new_state.is_audible());
        assert_eq!(new_state, crate::domain::audio::AudioState::Muted);
    }

    #[test]
    fn double_toggle_returns_to_initial() {
        // Muted → toggle → Live → toggle → Muted
        let pipeline = build_volume_pipeline();
        let ctrl = AudioController::new(
            pipeline,
            AudioConfig { enabled: false, volume: 0.8 },
        );
        assert!(!ctrl.state().is_audible());
        let _ = ctrl.toggle_mute();
        assert!(ctrl.state().is_audible());
        let _ = ctrl.toggle_mute();
        assert!(!ctrl.state().is_audible());
    }

    #[test]
    fn toggle_does_not_panic_when_element_missing() {
        // Pipeline without the `audio_volume`
        // element — the controller should
        // silently log a warning, not panic.
        gst::init().expect("gst::init");
        let pipeline = gst::Pipeline::new(None);
        let ctrl = AudioController::new(
            pipeline,
            AudioConfig { enabled: true, volume: 0.8 },
        );
        let new_state = ctrl.toggle_mute();
        // The state still toggles (it's
        // app-side state) — the warning is
        // about the missing element, not the
        // state machine.
        assert!(!new_state.is_audible());
    }

    #[test]
    fn toggle_mute_pushes_volume_to_pipeline() {
        // Real integration: build a
        // pipeline with the `volume`
        // element, set it to PLAYING, and
        // confirm that `toggle_mute` actually
        // changes the element's `volume`
        // property.
        let pipeline = build_volume_pipeline();
        pipeline
            .set_state(gst::State::Playing)
            .expect("set Playing");
        // Give the pipeline a moment to
        // actually transition.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let ctrl = AudioController::new(
            pipeline.clone(),
            AudioConfig { enabled: true, volume: 0.8 },
        );
        // Mute.
        let _ = ctrl.toggle_mute();
        // Read the volume back.
        let vol_elem = pipeline
            .by_name(AUDIO_VOLUME_NAME)
            .expect("audio_volume in pipeline");
        let v: f64 = vol_elem.property("volume");
        assert_eq!(v, 0.0, "expected muted (0.0), got {v}");
        // Unmute.
        let _ = ctrl.toggle_mute();
        let v: f64 = vol_elem.property("volume");
        assert!((v - 0.8).abs() < 1e-6, "expected 0.8, got {v}");
        // Tear down.
        let _ = pipeline.set_state(gst::State::Null);
    }

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
