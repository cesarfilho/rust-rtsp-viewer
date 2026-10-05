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
use crate::domain::motion::MotionConfig;
use crate::domain::multi_stream::StreamQuality;
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
