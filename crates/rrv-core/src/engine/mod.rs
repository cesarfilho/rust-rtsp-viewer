//! The video engine: everything that keeps cameras running, with no UI.
//!
//! Nothing in here may depend on `iced` or import `crate::ui` — it is meant to
//! move into a headless daemon (ADR 0010). `tests/engine_isolation.rs` enforces
//! it. The UI reaches the engine through [`crate::ui`]'s re-exports.

pub mod backoff;
pub mod bridge;
pub mod clip;
pub mod pipeline;
pub mod playback;

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::{CameraConfig, LogsConfigFile};
use crate::domain::camera_status::{CameraStatus, StatusReading};
use crate::domain::motion::MotionConfig;
use crate::domain::multi_stream::StreamQuality;
use crate::domain::multi_stream::{MultiStreamConfig, desired_quality, stream_url_for_quality};
use crate::domain::notify::NotifyConfig;
use crate::domain::recording::RecordingConfig;
use crate::domain::timeline::EventType;
use crate::domain::zones::ZoneConfig;
use crate::infrastructure::reconnect::{ReconnectDecision, ReconnectState};
use crate::infrastructure::store::{StoreCmd, StoreHandle};
use crate::infrastructure::zone_state::ZonesFile;
use std::sync::atomic::Ordering;

use backoff::BackoffState;

/// Everything [`Engine::new`] needs from the configuration.
pub struct EngineSettings<'a> {
    pub cameras: &'a [CameraConfig],
    pub recording: &'a RecordingConfig,
    pub motion: MotionConfig,
    pub notify: NotifyConfig,
    pub logs: &'a LogsConfigFile,
    /// `[view] pause_hidden`.
    pub pause_hidden: bool,
    /// `[view] stagger_ms`.
    pub stagger: Duration,
    /// Persisted zones (`zones.toml`), keyed by camera display name.
    pub zones: &'a ZonesFile,
}

/// The engine's tick, in milliseconds (the window's `TICK_MS` is the same).
pub const TICK_MS: u64 = 100;

/// Unix time in milliseconds (the history database's clock).
pub(crate) fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
/// Slow work (latency, RTP stats, motion) runs on every Nth tick (~500 ms).
pub const SLOW_EVERY_N_TICKS: u64 = 5;

/// The motion / recording / notification settings the config asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfiguredBehaviour {
    pub motion_enabled: bool,
    pub motion_recording: bool,
    pub notify_enabled: bool,
    /// Pre-roll seconds the config asked for (kept so leaving display-only restores it).
    pub preroll_secs: u32,
    pub record_audio: bool,
}

/// How long a freshly (re)started pipeline shows `Connecting` instead of
/// flapping to `Reconnecting`/`Offline` while it hand-shakes.
pub const CONNECT_GRACE_SECS: u64 = 12;

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
    /// Display-only: decode to show, but never record, detect motion or
    /// notify. The window uses this while a daemon owns those jobs (spec
    /// `ux-daemon.md`); two owners would record everything twice.
    pub display_only: bool,
    /// The history database (daemon only): events and recorded segments.
    pub store: Option<StoreHandle>,
    /// What the configuration asked for, kept so leaving display-only restores it.
    pub configured: ConfiguredBehaviour,
    /// Display name per camera (for notification text).
    pub names: Vec<String>,
    /// Events raised since the host last called [`Engine::take_events`].
    pub events: Vec<EngineEvent>,
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
    /// Build the engine from the configuration. Pipelines are **not** started
    /// here (a dozen HLS streams all connecting during construction froze the
    /// window on launch): every camera is queued in index order and the host
    /// drains the queue one camera per `stagger`. Use [`Engine::set_start_order`]
    /// to start in another order.
    ///
    /// A camera whose bridge cannot be built is dropped from every per-camera
    /// vector, so indices stay in lockstep; read the surviving cameras back from
    /// `camera_configs` and `names`.
    pub fn new(settings: EngineSettings<'_>) -> Self {
        let _ = gstreamer::init();
        let EngineSettings {
            cameras,
            recording,
            motion,
            notify,
            logs,
            pause_hidden,
            stagger,
            zones,
        } = settings;

        // Per-camera log directory (default: ~/logs/rust-rtsp-viewer).
        let log_dir = crate::infrastructure::recording_paths::ensure_log_dir(
            logs.dir
                .as_deref()
                .unwrap_or(std::path::Path::new("~/logs/rust-rtsp-viewer")),
        )
        .unwrap_or_else(|_| std::path::PathBuf::from("logs"));
        let retention_days = logs.retention_days.unwrap_or(7);

        let mut kept: Vec<CameraConfig> = Vec::with_capacity(cameras.len());
        let mut names: Vec<String> = Vec::with_capacity(cameras.len());
        let mut bridges = Vec::with_capacity(cameras.len());
        // Labels must be unique: two cameras with the same (or `safe_filename`-
        // colliding) label would share one log file and fight over rotation.
        let mut used_labels = std::collections::HashSet::new();

        for cam in cameras {
            let mut bridge = match bridge::GStreamerBridge::new(1920, 1080) {
                Ok(b) => b,
                Err(e) => {
                    // Sem nome nem rótulo cai para a URL, que pode levar a senha.
                    let who = cam
                        .label
                        .clone()
                        .or_else(|| cam.name.clone())
                        .unwrap_or_else(|| crate::domain::redact::mask_credentials(&cam.url));
                    log::error!("Skipping camera {who:?}: {e}");
                    continue;
                }
            };
            bridge.recording_config = recording.clone();
            bridge.detect_enabled = motion.enabled;
            bridge.record_audio = recording.record_audio;
            bridge.preroll_secs = if recording.on_motion {
                recording.motion_pre_roll_secs
            } else {
                0
            };
            let base_label = cam
                .label
                .clone()
                .or_else(|| cam.name.clone())
                .unwrap_or_else(|| format!("Camera {}", kept.len() + 1));
            // Disambiguate on the `safe_filename` form (which is lossy), so
            // "Centro: A" and "Centro A" still get separate log files.
            let mut label = base_label.clone();
            let mut dup = 2;
            while !used_labels.insert(crate::infrastructure::recording_paths::safe_filename(
                &label,
            )) {
                label = format!("{base_label} ({dup})");
                dup += 1;
            }
            let log_path = crate::infrastructure::recording_paths::log_path(&log_dir, &label);
            if let Ok(logger) = crate::infrastructure::recording_paths::CameraLogger::new(
                log_path,
                &label,
                retention_days,
            ) {
                bridge.set_logger(Arc::new(logger));
            }
            bridges.push(Arc::new(Mutex::new(bridge)));
            kept.push(cam.clone());
            names.push(label);
        }

        let count = bridges.len();
        let motion_recording = recording.on_motion;
        if pause_hidden
            && crate::domain::motion::needs_background_watch(
                motion.enabled,
                motion_recording,
                notify.enabled,
            )
        {
            log::info!(
                "[view] pause_hidden is overridden: on_motion / notifications need every \
                 camera decoding so hidden cameras are not blind"
            );
        }
        let zone_configs = names.iter().map(|n| zones.zone_config_for(n)).collect();

        Engine {
            bridges,
            pause_hidden,
            stagger,
            start_queue: (0..count).collect(),
            next_start_at: Instant::now(),
            active_stream: vec![false; count],
            connecting_since: vec![None; count],
            reconnect_states: (0..count).map(|_| ReconnectState::new(15)).collect(),
            backoff_states: (0..count).map(|_| BackoffState::new()).collect(),
            stream_quality: vec![StreamQuality::Main; count],
            display_only: false,
            store: None,
            configured: ConfiguredBehaviour {
                motion_enabled: motion.enabled,
                motion_recording,
                notify_enabled: notify.enabled,
                preroll_secs: if recording.on_motion {
                    recording.motion_pre_roll_secs
                } else {
                    0
                },
                record_audio: recording.record_audio,
            },
            names,
            events: Vec::new(),
            status: vec![CameraStatus::Connecting; count],
            camera_configs: kept,
            camera_enabled: vec![true; count],
            motion_config: motion,
            prev_motion_frames: vec![None; count],
            motion_active: vec![false; count],
            motion_recording,
            motion_post_roll_secs: recording.motion_post_roll_secs,
            last_motion_at: vec![None; count],
            auto_recording: vec![false; count],
            zones: zone_configs,
            notify,
            notify_last: HashMap::new(),
        }
    }

    /// Start the queued cameras in `order` (camera indices) instead of index
    /// order, so the page the user sees first comes up first.
    pub fn set_start_order(&mut self, order: &[usize]) {
        self.start_queue = order
            .iter()
            .copied()
            .filter(|&i| i < self.bridges.len())
            .collect();
    }

    /// Number of cameras the engine runs.
    pub fn camera_count(&self) -> usize {
        self.bridges.len()
    }
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

    /// Raise an event. Applies the notification policy: per (camera, kind)
    /// cooldown, so a flapping camera does not spam.
    pub fn emit(&mut self, camera: usize, kind: EventType, detail: Option<String>) {
        // Display-only: the daemon's events are the truth; this engine's own
        // would duplicate them on the timeline.
        if self.display_only {
            return;
        }
        if let Some(store) = &self.store {
            store.send(StoreCmd::Event {
                camera: self.names.get(camera).cloned().unwrap_or_default(),
                ts: now_ms(),
                kind: kind.slug().to_string(),
                label: detail.clone().unwrap_or_default(),
                score: None,
            });
        }
        let notification = self.notification_for(camera, kind, detail.as_deref());
        self.events.push(EngineEvent {
            camera,
            kind,
            detail,
            notification,
        });
    }

    fn notification_for(
        &mut self,
        camera: usize,
        kind: EventType,
        detail: Option<&str>,
    ) -> Option<(String, String)> {
        if !self.notify.enabled {
            return None;
        }
        let name = self
            .names
            .get(camera)
            .cloned()
            .unwrap_or_else(|| format!("Câmera {}", camera + 1));
        let message = crate::domain::notify::message_for(kind, &name, detail)?;
        let key = (camera, kind.label());
        let since = self.notify_last.get(&key).map(|t| t.elapsed().as_secs());
        if !crate::domain::notify::cooldown_elapsed(since, self.notify.cooldown_secs) {
            return None;
        }
        self.notify_last.insert(key, Instant::now());
        Some(message)
    }

    /// Hand the pending events to the host and clear them.
    pub fn take_events(&mut self) -> Vec<EngineEvent> {
        std::mem::take(&mut self.events)
    }

    /// Start or stop recording on camera `i`; returns whether it is recording
    /// now. A recording is taken from the decoded frames, so starting one on the
    /// sub-stream would save a low-resolution file: move to the main stream
    /// first (`wanted_quality` then leaves it alone until the recording stops).
    pub fn toggle_recording(&mut self, i: usize) -> Result<bool, String> {
        self.toggle_recording_as(i, "manual")
    }

    /// Like [`Engine::toggle_recording`], saying why a recording that starts does
    /// (`"motion"` or `"manual"`): retention treats them differently.
    fn toggle_recording_as(&mut self, i: usize, mode: &'static str) -> Result<bool, String> {
        if self.display_only {
            return Err("a gravação é feita pelo daemon".into());
        }
        let starting = !self.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_recording();
        if starting && self.stream_quality.get(i) == Some(&StreamQuality::Sub) {
            self.restart_stream(i, StreamQuality::Main);
        }
        let recording = {
            let mut b = self.bridges[i].lock().unwrap_or_else(|e| e.into_inner());
            b.recording_mode = mode;
            b.toggle_recording()?
        };
        self.status[i] = if recording {
            CameraStatus::Recording
        } else {
            CameraStatus::Live
        };
        let kind = if recording {
            EventType::RecordingStart
        } else {
            EventType::RecordingStop
        };
        self.emit(i, kind, None);
        Ok(recording)
    }

    /// Sample camera `i`'s detection frame and compare it with the previous
    /// one. Raises a `Motion` event on the rising edge.
    pub fn detect_motion(&mut self, i: usize) {
        if !self.motion_config.enabled
            || !matches!(self.status[i], CameraStatus::Live | CameraStatus::Recording)
        {
            self.prev_motion_frames[i] = None;
            self.motion_active[i] = false;
            return;
        }
        // The reduced detection branch (~320 px), not the full-resolution display
        // frame: same answer for a fraction of the pixels.
        let frame = self.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .capture_detect_frame();
        let Some((curr, width, height)) = frame else {
            return;
        };
        // Same allocation as last time = the branch has not produced a new frame
        // yet; diffing a frame with itself would read as "stillness".
        if self.prev_motion_frames[i]
            .as_ref()
            .is_some_and(|prev| prev.as_ptr() == curr.as_ptr() && prev.len() == curr.len())
        {
            return;
        }
        let zones = self.zones.get(i).filter(|z| z.has_active());
        let result = self.prev_motion_frames[i].as_ref().and_then(|prev| {
            crate::domain::motion::detect_motion(
                prev,
                &curr,
                width as usize,
                height as usize,
                &self.motion_config,
                zones,
            )
        });
        self.prev_motion_frames[i] = Some(curr);
        let Some(result) = result else {
            return;
        };
        log::debug!(
            "Motion sample, camera {i}: {:.1}% changed{}",
            result.motion_level * 100.0,
            if result.global_change {
                " (global change, discarded)"
            } else {
                ""
            }
        );
        if result.motion_active && !self.motion_active[i] {
            log::info!(
                "Motion on camera {i}: {:.1}% of the frame",
                result.motion_level * 100.0
            );
            self.emit(
                i,
                EventType::Motion,
                Some(format!("{:.1}% do quadro", result.motion_level * 100.0)),
            );
        }
        self.motion_active[i] = result.motion_active;
    }

    /// Start a recording when motion appears and stop it after the post-roll,
    /// but only ever stop a recording this trigger started.
    pub fn drive_motion_recording(&mut self, i: usize) {
        if !self.motion_recording {
            return;
        }
        let is_recording = self.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_recording();
        if !is_recording {
            // Stopped by the user (or a reconnect): the trigger no longer owns it.
            self.auto_recording[i] = false;
        }
        if self.motion_active[i] {
            self.last_motion_at[i] = Some(Instant::now());
        }
        let quiet = self.last_motion_at[i].map_or(u64::MAX, |t| t.elapsed().as_secs());
        let action = crate::domain::recording::motion_recording_action(
            is_recording,
            self.auto_recording[i],
            self.motion_active[i],
            quiet,
            self.motion_post_roll_secs,
        );
        match action {
            crate::domain::recording::MotionRecAction::None => {}
            crate::domain::recording::MotionRecAction::Start => {
                match self.toggle_recording_as(i, "motion") {
                    Ok(_) => self.auto_recording[i] = true,
                    Err(e) => log::warn!("Motion recording could not start on camera {i}: {e}"),
                }
            }
            crate::domain::recording::MotionRecAction::Stop => {
                if let Err(e) = self.toggle_recording(i) {
                    log::warn!("Motion recording could not stop on camera {i}: {e}");
                }
                self.auto_recording[i] = false;
            }
        }
    }

    /// One health tick for a running camera: drain its bus, measure fps and
    /// bitrate, refresh its status, raise online/offline events, run the
    /// connect grace and arm a retry when it is down. The host reacts to the
    /// returned [`CameraTick`] (sidebar row, and the reconnect itself, which it
    /// wraps with audio handling).
    ///
    /// The bridge guard is released before returning: `std::sync::Mutex` is not
    /// reentrant and the reconnect re-locks it.
    pub fn tick_camera(&mut self, i: usize, poll_slow_metrics: bool) -> CameraTick {
        let was_offline = matches!(
            self.status[i],
            CameraStatus::Offline | CameraStatus::Reconnecting
        );
        let label = self
            .names
            .get(i)
            .cloned()
            .unwrap_or_else(|| format!("Camera {}", i + 1));
        let is_uridecodebin = self
            .camera_configs
            .get(i)
            .map(|c| {
                c.use_uridecodebin.unwrap_or(false)
                    || c.url.starts_with("http://")
                    || c.url.starts_with("https://")
            })
            .unwrap_or(false);

        let (fps, reading, needs_reconnect, errored) = {
            let mut bridge = self.bridges[i].lock().unwrap_or_else(|e| e.into_inner());
            bridge.poll_bus();
            let fps = bridge.update_fps();
            let metrics = bridge.metrics().clone();
            metrics
                .current_fps_x1000
                .store((fps * 1000.0) as u64, Ordering::Relaxed);
            metrics.is_live.store(bridge.is_live(), Ordering::Relaxed);
            bridge.discover_rtp_jitterbuffers();
            bridge.discover_decoder();
            if poll_slow_metrics {
                bridge.query_latency();
                bridge.poll_rtp_stats();
            }
            let reading = bridge.sample_status(&self.status[i]);
            if let Some(status) = reading.as_ref().and_then(|r| r.status.clone()) {
                self.status[i] = status;
            }

            let is_live = bridge.is_live();
            let backoff_due = self.backoff_states[i].is_due();
            let decision = self.reconnect_states[i].tick(
                is_live,
                is_uridecodebin,
                Some(fps),
                backoff_due,
                &label,
            );
            if is_live && fps > 0.0 && matches!(decision, ReconnectDecision::None) {
                self.backoff_states[i].record_success();
            }
            let errored = bridge
                .error_message
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
            let needs_reconnect = match decision {
                ReconnectDecision::Reconnect(reason) => {
                    log::info!("[{label}] Reconnecting: {reason}");
                    true
                }
                ReconnectDecision::None => false,
            };
            (fps, reading, needs_reconnect, errored)
        };

        // A pipeline we started in the last few seconds shows CONNECTING rather
        // than flapping to RECONNECTING/OFFLINE while it hand-shakes. Cleared once
        // it actually goes live. This runs *before* the events below: a camera
        // that is still hand-shaking is not offline, so it must not raise an
        // Offline event (and a desktop notification) on every start or sub/main
        // switch.
        match self.connecting_since[i] {
            Some(_) if self.status[i] == CameraStatus::Live => {
                self.connecting_since[i] = None;
            }
            Some(t) if t.elapsed().as_secs() < CONNECT_GRACE_SECS => {
                if matches!(
                    self.status[i],
                    CameraStatus::Offline | CameraStatus::Reconnecting
                ) {
                    self.status[i] = CameraStatus::Connecting;
                }
            }
            Some(_) => self.connecting_since[i] = None,
            None => {}
        }

        // Log connectivity transitions once the status has settled.
        let now_offline = matches!(
            self.status[i],
            CameraStatus::Offline | CameraStatus::Reconnecting
        );
        if was_offline && !now_offline {
            self.emit(i, EventType::Online, None);
        } else if !was_offline && now_offline {
            self.emit(i, EventType::Offline, None);
        }

        // Down with no retry pending: schedule one. A pipeline that reported an
        // error is dead, so it retries right away; otherwise wait out the
        // connect grace. Without this a camera that fails right after starting
        // is stuck on "Reconectando" forever (the watchdog only covers live
        // RTSP stalls).
        let down = matches!(
            self.status[i],
            CameraStatus::Offline | CameraStatus::Reconnecting
        );
        if !needs_reconnect
            && BackoffState::should_schedule_retry(
                self.connecting_since[i].is_some(),
                down,
                errored,
            )
        {
            self.backoff_states[i].arm_if_idle();
        }

        CameraTick {
            fps,
            reading,
            needs_reconnect,
        }
    }

    /// Motion detection and the motion-triggered recording, run at ~2 Hz.
    pub fn tick_motion(&mut self, i: usize) {
        self.detect_motion(i);
        self.drive_motion_recording(i);
    }

    /// Switch a camera on or off. Off: stop its pipeline and show `Disabled`.
    /// On: leave the actual (re)start to the host's reconcile, which only spins
    /// it up if the camera is wanted (on the visible page, or always when
    /// something reacts to motion).
    pub fn set_camera_enabled(&mut self, i: usize, enabled: bool) {
        if i >= self.camera_enabled.len() {
            return;
        }
        self.camera_enabled[i] = enabled;
        self.active_stream[i] = false;
        self.backoff_states[i] = BackoffState::new();
        self.reconnect_states[i].reset();
        if enabled {
            self.status[i] = CameraStatus::Connecting;
        } else {
            self.bridges[i]
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .stop();
            self.connecting_since[i] = None;
            self.start_queue.retain(|&q| q != i);
            self.status[i] = CameraStatus::Disabled;
        }
    }

    /// One tick of the headless engine (the daemon calls this every
    /// [`TICK_MS`] ms); the window does the same work inside `update_frame`,
    /// interleaved with view math.
    ///
    /// With no window there is no "visible page": every enabled camera is
    /// wanted, and starts are still staggered. A camera runs on the stream
    /// `wanted_quality` picks (sub when it has one, main while recording).
    /// Returns the events raised during the tick.
    pub fn step(&mut self, tick: u64) -> Vec<EngineEvent> {
        let slow = tick.is_multiple_of(SLOW_EVERY_N_TICKS);
        let want = self.enabled_cameras();
        for i in self.reconcile(&want) {
            self.pause_stream(i);
        }
        if let Some(i) = self.pop_next_start(&want) {
            let quality = self.wanted_quality(i, false);
            self.restart_stream(i, quality);
        }
        for i in want {
            if !self.active_stream[i] {
                continue;
            }
            let label = self.names[i].clone();
            if self.tick_camera(i, slow).needs_reconnect {
                self.reconnect(i, &label);
            }
            if slow {
                self.tick_motion(i);
            }
        }
        self.take_events()
    }

    /// Stop every pipeline, finalising recordings in progress (the muxer writes
    /// its trailer on EOS; a process killed without this leaves an empty,
    /// unplayable file). Blocks until the files are closed.
    pub fn shutdown(&mut self) {
        for i in 0..self.bridges.len() {
            self.bridges[i]
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .stop();
            self.active_stream[i] = false;
            self.auto_recording[i] = false;
            self.connecting_since[i] = None;
            if self.status[i] != CameraStatus::Disabled {
                self.status[i] = CameraStatus::Paused;
            }
        }
        self.start_queue.clear();
    }

    /// Switch between doing everything (`false`) and display-only (`true`).
    ///
    /// Display-only turns off motion detection, motion recording and
    /// notifications and builds no recording or detection branch; leaving it
    /// restores what the configuration asked for. Running streams are rebuilt
    /// so their branches match the new mode.
    /// Keep the history (events and recorded segments) in `store`. Call before
    /// the first `step`.
    pub fn set_store(&mut self, store: StoreHandle) {
        for (i, b) in self.bridges.iter().enumerate() {
            b.lock().unwrap_or_else(|e| e.into_inner()).store =
                Some((store.clone(), self.names[i].clone()));
        }
        self.store = Some(store);
    }

    /// No window will ever show these cameras (the daemon): the pipelines skip
    /// the full-frame RGBA conversion and copy. Call before the first `step`;
    /// pipelines already running keep what they were built with.
    pub fn set_headless(&mut self) {
        for b in &self.bridges {
            b.lock().unwrap_or_else(|e| e.into_inner()).headless = true;
        }
    }

    pub fn set_display_only(&mut self, display_only: bool) {
        if self.display_only == display_only {
            return;
        }
        self.display_only = display_only;
        let c = self.configured;
        self.motion_config.enabled = c.motion_enabled && !display_only;
        self.motion_recording = c.motion_recording && !display_only;
        self.notify.enabled = c.notify_enabled && !display_only;
        for b in &self.bridges {
            let mut b = b.lock().unwrap_or_else(|e| e.into_inner());
            b.detect_enabled = self.motion_config.enabled;
            // A window that only shows has no use for a ring of encoded video.
            b.preroll_secs = if display_only { 0 } else { c.preroll_secs };
            b.record_audio = c.record_audio && !display_only;
        }
        for i in 0..self.bridges.len() {
            self.prev_motion_frames[i] = None;
            self.motion_active[i] = false;
            self.auto_recording[i] = false;
            if self.active_stream[i] {
                let quality = self.stream_quality[i];
                self.restart_stream(i, quality);
            }
        }
    }
}

/// Something that happened in the engine that the host should record and,
/// maybe, announce. The engine never talks to the desktop itself (there is no
/// desktop in a container): it decides *whether* and *what* to announce, and
/// the host (the window today, the daemon's webhook later) delivers it.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineEvent {
    pub camera: usize,
    pub kind: EventType,
    pub detail: Option<String>,
    /// `(title, body)` when the notification policy says to announce it
    /// (enabled, the kind is worth a message, the cooldown has elapsed).
    pub notification: Option<(String, String)>,
}

/// The result of [`Engine::tick_camera`].
#[derive(Debug, Clone)]
pub struct CameraTick {
    pub fps: f64,
    /// Fresh fps/bitrate/status for the sidebar row; `None` for a disabled camera.
    pub reading: Option<StatusReading>,
    /// The watchdog or the backoff timer says the pipeline must be rebuilt.
    pub needs_reconnect: bool,
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
            display_only: false,
            store: None,
            configured: ConfiguredBehaviour {
                motion_enabled: false,
                motion_recording: false,
                notify_enabled: false,
                preroll_secs: 0,
                record_audio: false,
            },
            names: (0..n).map(|i| format!("cam{i}")).collect(),
            events: Vec::new(),
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

    fn feed(e: &Engine, i: usize, rgba: Vec<u8>) {
        *e.bridges[i].lock().unwrap().detect_frame.lock().unwrap() =
            Some(crate::engine::bridge::DetectFrame {
                rgba: bytes::Bytes::from(rgba),
                width: 20,
                height: 20,
            });
    }

    /// A 20×20 gray frame with the first `rows` rows turned white.
    fn frame(rows: usize) -> Vec<u8> {
        let mut f = vec![100u8; 20 * 20 * 4];
        for px in f.chunks_mut(4).take(rows * 20) {
            px[..3].copy_from_slice(&[255, 255, 255]);
        }
        f
    }

    fn live_motion_engine() -> Engine {
        let mut e = engine(1);
        e.status[0] = CameraStatus::Live;
        e.motion_config = MotionConfig {
            enabled: true,
            sample_stride: 1,
            ..MotionConfig::default()
        };
        e
    }

    #[test]
    fn motion_raises_one_event_on_the_rising_edge() {
        let mut e = live_motion_engine();
        feed(&e, 0, frame(0));
        e.detect_motion(0); // first sample: nothing to compare with
        feed(&e, 0, frame(4)); // 20% of the frame changes
        e.detect_motion(0);
        feed(&e, 0, frame(5));
        e.detect_motion(0); // still moving: no second event
        let events = e.take_events();
        assert_eq!(events.len(), 1, "got {events:?}");
        assert_eq!(events[0].kind, EventType::Motion);
        assert!(e.motion_active[0]);
    }

    #[test]
    fn a_whole_frame_change_is_not_motion() {
        let mut e = live_motion_engine();
        feed(&e, 0, frame(0));
        e.detect_motion(0);
        feed(&e, 0, frame(20)); // everything flips at once (IR cut, exposure)
        e.detect_motion(0);
        assert!(e.take_events().is_empty());
        assert!(!e.motion_active[0]);
    }

    #[test]
    fn a_camera_that_is_not_live_has_no_motion_state() {
        let mut e = live_motion_engine();
        e.status[0] = CameraStatus::Offline;
        e.motion_active[0] = true;
        e.prev_motion_frames[0] = Some(bytes::Bytes::from(frame(0)));
        feed(&e, 0, frame(4));
        e.detect_motion(0);
        assert!(!e.motion_active[0]);
        assert!(e.prev_motion_frames[0].is_none());
        assert!(e.take_events().is_empty());
    }

    #[test]
    fn the_same_detection_frame_is_not_compared_with_itself() {
        let mut e = live_motion_engine();
        feed(&e, 0, frame(0));
        e.detect_motion(0);
        // no new frame arrived: the slot still holds the same allocation
        e.detect_motion(0);
        e.detect_motion(0);
        assert!(e.take_events().is_empty());
        assert!(!e.motion_active[0]);
    }

    #[test]
    fn events_carry_a_notification_only_when_the_policy_allows() {
        let mut e = engine(1);
        e.emit(0, EventType::Motion, None);
        assert!(
            e.take_events()[0].notification.is_none(),
            "notifications are off"
        );

        e.notify.enabled = true;
        e.notify.cooldown_secs = 60;
        e.emit(0, EventType::Motion, Some("5% do quadro".into()));
        e.emit(0, EventType::Motion, None);
        let events = e.take_events();
        assert!(events[0].notification.is_some(), "first one is announced");
        assert!(
            events[1].notification.is_none(),
            "the second is inside the cooldown"
        );
    }

    #[test]
    fn take_events_clears_the_queue() {
        let mut e = engine(1);
        e.emit(0, EventType::Online, None);
        assert_eq!(e.take_events().len(), 1);
        assert!(e.take_events().is_empty());
    }

    #[test]
    fn toggling_recording_without_a_pipeline_fails_cleanly() {
        let mut e = engine(1);
        e.status[0] = CameraStatus::Live;
        assert!(e.toggle_recording(0).is_err());
        assert_eq!(e.status[0], CameraStatus::Live, "status is untouched");
        assert!(e.take_events().is_empty(), "no event for a failed toggle");
    }

    #[test]
    fn a_camera_that_went_down_raises_offline_and_arms_a_retry() {
        let mut e = engine(1);
        e.status[0] = CameraStatus::Live;
        e.active_stream[0] = true;
        // no pipeline: the bridge is neither live nor reconnecting → Offline
        let tick = e.tick_camera(0, false);
        assert_eq!(e.status[0], CameraStatus::Offline);
        assert!(!tick.needs_reconnect, "the retry waits for its timer");
        let events = e.take_events();
        assert_eq!(events.len(), 1, "got {events:?}");
        assert_eq!(events[0].kind, EventType::Offline);
        assert!(
            e.backoff_states[0].next_attempt.is_some(),
            "a camera that is down must arm a retry"
        );
    }

    #[test]
    fn a_just_started_camera_shows_connecting_not_offline() {
        let mut e = engine(1);
        e.status[0] = CameraStatus::Connecting;
        e.active_stream[0] = true;
        e.connecting_since[0] = Some(Instant::now());
        e.tick_camera(0, false);
        assert_eq!(
            e.status[0],
            CameraStatus::Connecting,
            "inside the connect grace"
        );
        assert!(e.take_events().is_empty());
        assert!(
            e.backoff_states[0].next_attempt.is_none(),
            "no retry while the camera is still hand-shaking"
        );
    }

    #[test]
    fn the_connect_grace_expires() {
        let mut e = engine(1);
        e.status[0] = CameraStatus::Connecting;
        e.active_stream[0] = true;
        e.connecting_since[0] = Some(Instant::now() - Duration::from_secs(CONNECT_GRACE_SECS + 1));
        e.tick_camera(0, false);
        assert!(e.connecting_since[0].is_none());
        assert_eq!(e.status[0], CameraStatus::Offline);
        let events = e.take_events();
        assert_eq!(
            events.len(),
            1,
            "one Offline once the grace is over: {events:?}"
        );
        assert_eq!(events[0].kind, EventType::Offline);
    }

    #[test]
    fn a_disabled_camera_reports_nothing() {
        let mut e = engine(1);
        e.status[0] = CameraStatus::Disabled;
        let tick = e.tick_camera(0, false);
        assert!(tick.reading.is_none());
        assert_eq!(e.status[0], CameraStatus::Disabled);
    }

    #[test]
    fn disabling_a_camera_stops_and_marks_it_disabled() {
        let mut e = engine(2);
        e.active_stream[0] = true;
        e.connecting_since[0] = Some(Instant::now());
        e.start_queue.push_back(0);
        e.set_camera_enabled(0, false);
        assert!(!e.camera_enabled[0]);
        assert!(!e.active_stream[0]);
        assert!(e.connecting_since[0].is_none());
        assert!(e.start_queue.is_empty());
        assert_eq!(e.status[0], CameraStatus::Disabled);
        assert_eq!(e.enabled_cameras(), vec![1]);
    }

    #[test]
    fn re_enabling_leaves_the_start_to_reconcile() {
        let mut e = engine(1);
        e.set_camera_enabled(0, false);
        e.set_camera_enabled(0, true);
        assert!(e.camera_enabled[0]);
        assert!(!e.active_stream[0], "it is not started here");
        assert_eq!(e.status[0], CameraStatus::Connecting);
        // the host's reconcile then queues it
        assert!(e.reconcile(&[0]).is_empty());
        assert_eq!(e.start_queue, VecDeque::from([0]));
    }

    #[test]
    fn toggling_an_unknown_camera_is_ignored() {
        let mut e = engine(1);
        e.set_camera_enabled(7, false);
        assert!(e.camera_enabled[0]);
    }

    // ---- Engine::new ----

    fn cams(toml_cams: &str) -> Vec<CameraConfig> {
        #[derive(serde::Deserialize)]
        struct W {
            cameras: Vec<CameraConfig>,
        }
        toml::from_str::<W>(toml_cams).unwrap().cameras
    }

    /// Build an engine in a throwaway log directory.
    fn built(cameras: &[CameraConfig], motion_on: bool, zones: &ZonesFile) -> Engine {
        let dir = std::env::temp_dir().join(format!("rrv-engine-new-{}", std::process::id()));
        let logs = LogsConfigFile {
            dir: Some(dir),
            ..LogsConfigFile::default()
        };
        Engine::new(EngineSettings {
            cameras,
            recording: &RecordingConfig::default(),
            motion: MotionConfig {
                enabled: motion_on,
                ..MotionConfig::default()
            },
            notify: NotifyConfig {
                enabled: false,
                ..NotifyConfig::default()
            },
            logs: &logs,
            pause_hidden: true,
            stagger: Duration::from_millis(250),
            zones,
        })
    }

    #[test]
    fn new_names_cameras_and_keeps_every_vec_in_lockstep() {
        let c = cams(
            "[[cameras]]\nurl = \"rtsp://a/s\"\nname = \"Portão\"\n\
             [[cameras]]\nurl = \"rtsp://b/s\"\nlabel = \"Garagem\"\n\
             [[cameras]]\nurl = \"rtsp://c/s\"\n",
        );
        let e = built(&c, false, &ZonesFile::default());
        assert_eq!(e.camera_count(), 3);
        assert_eq!(e.names, vec!["Portão", "Garagem", "Camera 3"]);
        for len in [
            e.active_stream.len(),
            e.connecting_since.len(),
            e.reconnect_states.len(),
            e.backoff_states.len(),
            e.stream_quality.len(),
            e.status.len(),
            e.camera_configs.len(),
            e.camera_enabled.len(),
            e.prev_motion_frames.len(),
            e.zones.len(),
        ] {
            assert_eq!(len, 3);
        }
        assert!(e.status.iter().all(|s| *s == CameraStatus::Connecting));
    }

    #[test]
    fn new_disambiguates_duplicate_and_filename_colliding_labels() {
        let c = cams(
            "[[cameras]]\nurl = \"rtsp://a/s\"\nname = \"Cam\"\n\
             [[cameras]]\nurl = \"rtsp://b/s\"\nname = \"Cam\"\n\
             [[cameras]]\nurl = \"rtsp://c/s\"\nname = \"Centro: A\"\n\
             [[cameras]]\nurl = \"rtsp://d/s\"\nname = \"Centro A\"\n",
        );
        let e = built(&c, false, &ZonesFile::default());
        assert_eq!(e.names[0], "Cam");
        assert_eq!(e.names[1], "Cam (2)");
        assert_eq!(e.names[2], "Centro: A");
        assert_eq!(
            e.names[3], "Centro A (2)",
            "labels that sanitise to the same file name must not share a log"
        );
    }

    #[test]
    fn new_queues_every_camera_and_starts_none() {
        let c = cams("[[cameras]]\nurl = \"rtsp://a/s\"\n[[cameras]]\nurl = \"rtsp://b/s\"\n");
        let e = built(&c, false, &ZonesFile::default());
        assert_eq!(e.start_queue, VecDeque::from([0, 1]));
        assert!(
            e.active_stream.iter().all(|a| !a),
            "nothing starts in new()"
        );
        assert!(
            e.bridges
                .iter()
                .all(|b| b.lock().unwrap().pipeline().is_none())
        );
    }

    #[test]
    fn set_start_order_reorders_and_drops_unknown_indices() {
        let c = cams(
            "[[cameras]]\nurl = \"rtsp://a/s\"\n[[cameras]]\nurl = \"rtsp://b/s\"\n\
             [[cameras]]\nurl = \"rtsp://c/s\"\n",
        );
        let mut e = built(&c, false, &ZonesFile::default());
        e.set_start_order(&[2, 9, 0]);
        assert_eq!(e.start_queue, VecDeque::from([2, 0]));
    }

    #[test]
    fn new_enables_the_detection_branch_only_when_motion_is_on() {
        let c = cams("[[cameras]]\nurl = \"rtsp://a/s\"\n");
        let on = built(&c, true, &ZonesFile::default());
        let off = built(&c, false, &ZonesFile::default());
        assert!(on.bridges[0].lock().unwrap().detect_enabled);
        assert!(!off.bridges[0].lock().unwrap().detect_enabled);
    }

    #[test]
    fn new_loads_persisted_zones_by_camera_name() {
        let c = cams(
            "[[cameras]]\nurl = \"rtsp://a/s\"\nname = \"Portão\"\n\
             [[cameras]]\nurl = \"rtsp://b/s\"\nname = \"Garagem\"\n",
        );
        let mut zones = ZonesFile::default();
        zones.cameras.insert(
            "Garagem".into(),
            vec![crate::domain::zones::MotionZoneFile {
                name: "z".into(),
                vertices: vec![
                    crate::domain::zones::PointFile { x: 0.1, y: 0.1 },
                    crate::domain::zones::PointFile { x: 0.9, y: 0.1 },
                    crate::domain::zones::PointFile { x: 0.5, y: 0.9 },
                ],
                enabled: Some(true),
            }],
        );
        let e = built(&c, false, &zones);
        assert!(!e.zones[0].has_active(), "Portão has no zones");
        assert!(e.zones[1].has_active(), "Garagem's zone is found by name");
    }

    #[test]
    fn new_with_no_cameras_is_an_empty_engine() {
        let e = built(&[], false, &ZonesFile::default());
        assert_eq!(e.camera_count(), 0);
        assert!(e.start_queue.is_empty());
        assert!(e.enabled_cameras().is_empty());
    }

    // ---- display-only ----

    fn full_engine() -> Engine {
        let c = cams("[[cameras]]\nurl = \"rtsp://a/s\"\n[[cameras]]\nurl = \"rtsp://b/s\"\n");
        let dir = std::env::temp_dir().join(format!("rrv-engine-do-{}", std::process::id()));
        Engine::new(EngineSettings {
            cameras: &c,
            recording: &RecordingConfig {
                on_motion: true,
                ..RecordingConfig::default()
            },
            motion: MotionConfig {
                enabled: true,
                ..MotionConfig::default()
            },
            notify: NotifyConfig {
                enabled: true,
                ..NotifyConfig::default()
            },
            logs: &LogsConfigFile {
                dir: Some(dir),
                ..LogsConfigFile::default()
            },
            pause_hidden: true,
            stagger: Duration::from_millis(100),
            zones: &ZonesFile::default(),
        })
    }

    #[test]
    fn display_only_turns_off_motion_recording_and_notifications() {
        let mut e = full_engine();
        assert!(e.motion_config.enabled && e.motion_recording && e.notify.enabled);
        assert!(e.bridges.iter().all(|b| b.lock().unwrap().detect_enabled));
        assert!(
            e.must_run_everything(),
            "com on_motion todas as câmeras rodam"
        );

        e.set_display_only(true);
        assert!(!e.motion_config.enabled && !e.motion_recording && !e.notify.enabled);
        assert!(
            e.bridges.iter().all(|b| !b.lock().unwrap().detect_enabled),
            "nenhum ramo de detecção é montado"
        );
        assert!(
            !e.must_run_everything(),
            "sem gatilho de movimento, a visão decide quais câmeras rodam"
        );
    }

    #[test]
    fn leaving_display_only_restores_what_the_config_asked_for() {
        let mut e = full_engine();
        e.set_display_only(true);
        e.set_display_only(false);
        assert!(e.motion_config.enabled && e.motion_recording && e.notify.enabled);
        assert!(e.bridges.iter().all(|b| b.lock().unwrap().detect_enabled));
        assert!(!e.display_only);
    }

    #[test]
    fn display_only_refuses_to_record_and_raises_no_events() {
        let mut e = full_engine();
        e.set_display_only(true);
        let err = e.toggle_recording(0).unwrap_err();
        assert!(err.contains("daemon"), "{err}");
        e.emit(0, EventType::Offline, None);
        assert!(e.take_events().is_empty(), "os eventos são os do daemon");
    }

    #[test]
    fn switching_modes_twice_is_a_no_op_and_clears_motion_state() {
        let mut e = full_engine();
        e.motion_active[0] = true;
        e.auto_recording[1] = true;
        e.set_display_only(false); // já era false: nada muda
        assert!(e.motion_active[0] && e.auto_recording[1]);
        e.set_display_only(true);
        assert!(!e.motion_active[0] && !e.auto_recording[1]);
    }
}
