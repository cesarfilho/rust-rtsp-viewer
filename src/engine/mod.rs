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

    /// Rebuild a camera's pipeline, resuming an in-progress recording into a
    /// fresh segment. The caller handles what is not the engine's (audio, UI
    /// events) using the returned [`ReconnectOutcome`].
    pub fn reconnect(&mut self, i: usize, label: &str) -> Option<ReconnectOutcome> {
        let cfg = self.camera_config_for(i)?;
        let (restarted, was_recording) = {
            let mut bridge = self.bridges[i].lock().unwrap_or_else(|e| e.into_inner());
            let was_recording = bridge.is_recording();
            bridge.note_reconnect();
            // `stop` finalises the current segment; the recording resumes into a
            // fresh segment below rather than silently ending at the outage.
            bridge.stop();
            match bridge.start_from_config(&cfg) {
                Ok(()) => {
                    if was_recording && let Err(e) = bridge.start_recording() {
                        log::warn!("[{label}] Could not resume recording: {e}");
                    }
                    (true, was_recording)
                }
                Err(e) => {
                    log::warn!("[{label}] Reconnect failed: {e}");
                    (false, was_recording)
                }
            }
        };

        if restarted {
            // One rebuild per backoff period, and a fresh connect grace for it.
            self.backoff_states[i].disarm();
            self.connecting_since[i] = Some(Instant::now());
        } else {
            // Only a genuine failure feeds the exponential backoff; a successful
            // rebuild must not inflate `consecutive_failures` (which would drag
            // the reconnect delay up and, before the cap, could overflow it).
            self.backoff_states[i].record_failure();
        }
        self.status[i] = CameraStatus::Reconnecting;
        Some(ReconnectOutcome {
            restarted,
            resumed_recording: restarted && was_recording,
        })
    }

    /// The next queued camera that may start now, if any: at most one per
    /// `stagger`, so a launch or page flip never opens a dozen streams at once.
    /// `want` is what should be running; queued cameras no longer in it (the
    /// page moved again before their turn) are dropped.
    pub fn pop_next_start(&mut self, want: &[usize]) -> Option<usize> {
        if self.start_queue.is_empty() || Instant::now() < self.next_start_at {
            return None;
        }
        while let Some(i) = self.start_queue.pop_front() {
            if !self.camera_enabled.get(i).copied().unwrap_or(false) || self.active_stream[i] {
                continue;
            }
            if !want.contains(&i) {
                continue;
            }
            self.next_start_at = Instant::now() + self.stagger;
            return Some(i);
        }
        None
    }

    /// Reconcile which pipelines should run with `want`. Cameras that should
    /// run but do not are queued to start; the cameras returned are running
    /// but no longer wanted — the caller pauses them (it also silences their
    /// audio).
    pub fn reconcile(&mut self, want: &[usize]) -> Vec<usize> {
        let mut to_pause = Vec::new();
        for i in 0..self.bridges.len() {
            if !self.camera_enabled[i] {
                continue;
            }
            let should_run = want.contains(&i);
            if should_run && !self.active_stream[i] {
                if !self.start_queue.contains(&i) {
                    self.start_queue.push_back(i);
                }
                if self.status[i] == CameraStatus::Paused {
                    self.status[i] = CameraStatus::Connecting;
                }
            } else if !should_run && self.active_stream[i] {
                to_pause.push(i);
            } else if !should_run && !self.active_stream[i] {
                // Off-page and not running: settle its placeholder on PAUSED
                // (unless it never started and is still in the launch queue).
                self.start_queue.retain(|&q| q != i);
                if self.status[i] == CameraStatus::Connecting {
                    self.status[i] = CameraStatus::Paused;
                }
            }
        }
        to_pause
    }
}

/// What a [`Engine::reconnect`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectOutcome {
    /// The pipeline was rebuilt (`false`: the rebuild failed and backoff grew).
    pub restarted: bool,
    /// A recording that was running resumed into a fresh segment.
    pub resumed_recording: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::bridge::GStreamerBridge;

    /// An engine with `n` cameras and no pipelines (a bridge without a
    /// pipeline is inert), `pause_hidden` on and nothing reacting to motion.
    fn engine(n: usize) -> Engine {
        let _ = gstreamer::init();
        let cfg: CameraConfig = toml::from_str("url = \"rtsp://h/s\"").unwrap();
        Engine {
            bridges: (0..n)
                .map(|_| Arc::new(Mutex::new(GStreamerBridge::new(64, 36).unwrap())))
                .collect(),
            pause_hidden: true,
            stagger: Duration::from_millis(500),
            start_queue: VecDeque::new(),
            next_start_at: Instant::now(),
            active_stream: vec![false; n],
            connecting_since: vec![None; n],
            reconnect_states: (0..n).map(|_| ReconnectState::new(15)).collect(),
            backoff_states: (0..n).map(|_| BackoffState::new()).collect(),
            stream_quality: vec![StreamQuality::Main; n],
            status: vec![CameraStatus::Connecting; n],
            camera_configs: vec![cfg; n],
            camera_enabled: vec![true; n],
            motion_config: MotionConfig {
                enabled: false,
                ..MotionConfig::default()
            },
            prev_motion_frames: vec![None; n],
            motion_active: vec![false; n],
            motion_recording: false,
            motion_post_roll_secs: 15,
            last_motion_at: vec![None; n],
            auto_recording: vec![false; n],
            zones: vec![ZoneConfig::default(); n],
            notify: NotifyConfig {
                enabled: false,
                ..NotifyConfig::default()
            },
            notify_last: HashMap::new(),
        }
    }

    #[test]
    fn reconcile_queues_a_wanted_camera_that_is_not_running() {
        let mut e = engine(3);
        e.status[1] = CameraStatus::Paused;
        let to_pause = e.reconcile(&[1]);
        assert!(to_pause.is_empty());
        assert_eq!(e.start_queue, VecDeque::from([1]));
        assert_eq!(e.status[1], CameraStatus::Connecting, "Paused → Connecting");
    }

    #[test]
    fn reconcile_does_not_queue_twice() {
        let mut e = engine(2);
        e.reconcile(&[0]);
        e.reconcile(&[0]);
        assert_eq!(e.start_queue.len(), 1);
    }

    #[test]
    fn reconcile_returns_running_cameras_that_are_no_longer_wanted() {
        let mut e = engine(3);
        e.active_stream = vec![true, true, false];
        assert_eq!(e.reconcile(&[0]), vec![1]);
    }

    #[test]
    fn reconcile_settles_an_idle_unwanted_camera_on_paused() {
        let mut e = engine(2);
        e.start_queue.push_back(1);
        // camera 1 never started and is no longer wanted
        e.reconcile(&[0]);
        assert!(!e.start_queue.contains(&1));
        assert_eq!(e.status[1], CameraStatus::Paused);
    }

    #[test]
    fn reconcile_leaves_disabled_cameras_alone() {
        let mut e = engine(2);
        e.camera_enabled[0] = false;
        e.status[0] = CameraStatus::Disabled;
        e.reconcile(&[0, 1]);
        assert_eq!(e.status[0], CameraStatus::Disabled);
        assert!(!e.start_queue.contains(&0));
    }

    #[test]
    fn starts_are_staggered_one_per_interval() {
        let mut e = engine(3);
        e.start_queue = VecDeque::from([0, 1, 2]);
        assert_eq!(e.pop_next_start(&[0, 1, 2]), Some(0));
        // the stagger has not elapsed: nothing else may start yet
        assert_eq!(e.pop_next_start(&[0, 1, 2]), None);
        e.next_start_at = Instant::now();
        assert_eq!(e.pop_next_start(&[0, 1, 2]), Some(1));
    }

    #[test]
    fn a_queued_camera_that_is_no_longer_wanted_is_skipped() {
        let mut e = engine(3);
        e.start_queue = VecDeque::from([0, 1]);
        // the page moved again before camera 0's turn
        assert_eq!(e.pop_next_start(&[1]), Some(1));
    }

    #[test]
    fn running_and_disabled_cameras_are_not_started_again() {
        let mut e = engine(3);
        e.active_stream[0] = true;
        e.camera_enabled[1] = false;
        e.start_queue = VecDeque::from([0, 1, 2]);
        assert_eq!(e.pop_next_start(&[0, 1, 2]), Some(2));
    }

    #[test]
    fn hidden_cameras_run_when_something_reacts_to_motion() {
        let mut e = engine(2);
        assert!(!e.must_run_everything(), "pause_hidden alone may pause");
        e.motion_config.enabled = true;
        assert!(
            !e.must_run_everything(),
            "motion without a reaction is only an indicator"
        );
        e.motion_recording = true;
        assert!(e.must_run_everything(), "on_motion needs every camera");
        e.motion_recording = false;
        e.notify.enabled = true;
        assert!(e.must_run_everything(), "notifications need every camera");
    }

    #[test]
    fn everything_runs_when_pause_hidden_is_off() {
        let mut e = engine(2);
        e.pause_hidden = false;
        assert!(e.must_run_everything());
    }

    #[test]
    fn a_camera_without_a_sub_stream_always_wants_main() {
        let e = engine(1);
        assert_eq!(e.wanted_quality(0, false), StreamQuality::Main);
        assert_eq!(e.wanted_quality(0, true), StreamQuality::Main);
    }

    #[test]
    fn pausing_a_camera_marks_it_paused_and_dequeues_it() {
        let mut e = engine(2);
        e.active_stream[0] = true;
        e.start_queue.push_back(0);
        e.pause_stream(0);
        assert!(!e.active_stream[0]);
        assert_eq!(e.status[0], CameraStatus::Paused);
        assert!(e.start_queue.is_empty());
    }

    #[test]
    fn only_enabled_cameras_are_listed() {
        let mut e = engine(3);
        e.camera_enabled[1] = false;
        assert_eq!(e.enabled_cameras(), vec![0, 2]);
    }
}
