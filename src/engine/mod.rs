//! The video engine: everything that keeps cameras running, with no UI.
//!
//! Nothing in here may depend on `iced` or import `crate::ui` — it is meant to
//! move into a headless daemon (ADR 0010). `tests/engine_isolation.rs` enforces
//! it. The UI reaches the engine through [`crate::ui`]'s re-exports.

pub mod backoff;
pub mod bridge;
pub mod pipeline;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::CameraConfig;
use crate::domain::camera_status::CameraStatus;
use crate::domain::motion::MotionConfig;
use crate::domain::multi_stream::StreamQuality;
use crate::domain::multi_stream::{MultiStreamConfig, desired_quality, stream_url_for_quality};
use crate::domain::notify::NotifyConfig;
use crate::domain::zones::ZoneConfig;
use crate::infrastructure::reconnect::ReconnectState;

use backoff::BackoffState;

/// Per-camera runtime state of the video engine. Every `Vec` is indexed by
/// camera and kept in lockstep. The UI owns one and reads it; the orchestration
/// that mutates it is being moved here from `ui/update.rs` (plan task 2.5.2).
pub struct Engine {
    /// One GStreamer bridge per camera (shared with the UI's video widgets).
    pub bridges: Vec<Arc<Mutex<bridge::GStreamerBridge>>>,
    /// `[view] pause_hidden` — stop pipelines for cameras off the visible page.
    pub pause_hidden: bool,
    /// Spacing between staggered pipeline starts.
    pub stagger: Duration,
    /// Camera indices waiting for their pipeline to be started (initial launch and page flips both feed this).
    pub start_queue: VecDeque<usize>,
    /// Earliest instant the next queued camera may start.
    pub next_start_at: Instant,
    /// Whether each camera's video pipeline is currently running.
    pub active_stream: Vec<bool>,
    /// When each camera's pipeline was last (re)started; `None` once it has gone live or was never started.
    pub connecting_since: Vec<Option<Instant>>,
    /// Per-camera FPS watchdog + reconnect decision.
    pub reconnect_states: Vec<ReconnectState>,
    /// Per-camera reconnect backoff.
    pub backoff_states: Vec<BackoffState>,
    /// What each camera is doing right now (connecting, live, recording, ...). The UI's sidebar
    /// rows mirror this after every update.
    pub status: Vec<CameraStatus>,
    /// The configuration each camera was started from.
    pub camera_configs: Vec<CameraConfig>,
    /// Which stream each camera is running on (`Sub` only for cameras with a `sub_url`).
    pub stream_quality: Vec<StreamQuality>,
    /// Cameras the user switched off stay stopped.
    pub camera_enabled: Vec<bool>,
    /// `[motion]` detector tuning.
    pub motion_config: MotionConfig,
    /// Previous sampled detection frame per camera, for frame differencing.
    pub prev_motion_frames: Vec<Option<bytes::Bytes>>,
    /// Whether motion was active at the last sample (events fire on the rising edge).
    pub motion_active: Vec<bool>,
    /// `[recording] on_motion`.
    pub motion_recording: bool,
    /// `[recording] motion_post_roll_secs`.
    pub motion_post_roll_secs: u32,
    /// Last instant motion was seen per camera (drives the post-roll).
    pub last_motion_at: Vec<Option<Instant>>,
    /// True for recordings the motion trigger started, so it never stops a manual one.
    pub auto_recording: Vec<bool>,
    /// Motion zones per camera (empty = the whole frame counts).
    pub zones: Vec<ZoneConfig>,
    /// Desktop-notification policy.
    pub notify: NotifyConfig,
    /// Last desktop notification per (camera, event kind), for the cooldown.
    pub notify_last: HashMap<(usize, &'static str), Instant>,
}

impl Engine {
    /// Cameras the user has not switched off.
    pub fn enabled_cameras(&self) -> Vec<usize> {
        (0..self.bridges.len())
            .filter(|&i| self.camera_enabled[i])
            .collect()
    }

    /// Whether every enabled camera must keep decoding no matter what is on
    /// screen: either `pause_hidden` is off, or something reacts to motion (a
    /// hidden camera is blind, so it could not start a recording or an alert).
    pub fn must_run_everything(&self) -> bool {
        !self.pause_hidden
            || crate::domain::motion::needs_background_watch(
                self.motion_config.enabled,
                self.motion_recording,
                self.notify.enabled,
            )
    }

    /// The camera config with `url` swapped for the sub-stream when that is the
    /// stream it is running on.
    pub fn camera_config_for(&self, i: usize) -> Option<CameraConfig> {
        let mut cfg = self.camera_configs.get(i)?.clone();
        let multi = MultiStreamConfig {
            sub_stream_url: cfg.sub_url.clone(),
            default_quality: StreamQuality::Main,
        };
        let quality = self.stream_quality.get(i).copied().unwrap_or_default();
        cfg.url = stream_url_for_quality(&cfg.url, quality, &multi);
        Some(cfg)
    }

    /// The stream camera `i` should be on. `large_view` says whether it fills
    /// the view (spotlight, flex main, or alone on the page) — the only part of
    /// this decision that belongs to the UI.
    pub fn wanted_quality(&self, i: usize, large_view: bool) -> StreamQuality {
        let current = self.stream_quality.get(i).copied().unwrap_or_default();
        let has_sub = self
            .camera_configs
            .get(i)
            .is_some_and(|c| c.sub_url.is_some());
        if !has_sub {
            return StreamQuality::Main;
        }
        let recording = self.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_recording();
        desired_quality(has_sub, recording, current, large_view)
    }

    /// (Re)build a camera's pipeline on `quality`. `start_*` stops the old
    /// pipeline first, so this doubles as the sub/main switch.
    pub fn restart_stream(&mut self, i: usize, quality: StreamQuality) {
        if i >= self.stream_quality.len() {
            return;
        }
        self.stream_quality[i] = quality;
        let Some(cfg) = self.camera_config_for(i) else {
            return;
        };
        let result = {
            let mut bridge = self.bridges[i].lock().unwrap_or_else(|e| e.into_inner());
            bridge.start_from_config(&cfg)
        };
        self.active_stream[i] = true;
        self.connecting_since[i] = Some(Instant::now());
        self.reconnect_states[i].reset();
        self.backoff_states[i] = BackoffState::new();
        self.status[i] = if result.is_err() {
            CameraStatus::Offline
        } else {
            CameraStatus::Connecting
        };
        if let Err(e) = result {
            log::warn!("Could not start camera {i}: {e}");
        }
    }

    /// Stop a camera's video pipeline because it is not needed right now.
    /// It shows `Paused` (not a fault) and leaves the start queue.
    pub fn pause_stream(&mut self, i: usize) {
        self.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop();
        self.active_stream[i] = false;
        self.connecting_since[i] = None;
        self.reconnect_states[i].reset();
        self.backoff_states[i] = BackoffState::new();
        self.start_queue.retain(|&q| q != i);
        self.status[i] = CameraStatus::Paused;
    }
}
