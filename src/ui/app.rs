use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iced::Task;

use crate::domain::audio::{AudioConfig, AudioState};
use crate::domain::recording::RecordingConfig;
use crate::domain::snapshot::SnapshotConfig;
use crate::domain::view::ViewSettings;
use crate::infrastructure::audio::AudioLevelState;
use crate::infrastructure::reconnect::ReconnectState;

use super::bridge::GStreamerBridge;
use super::message::{LayoutMode, Message};
use super::sidebar;
use super::state::{BackoffState, ContextMenu, Toast};
use super::theme::Theme;
use super::video_widget::VideoWidget;

pub(crate) const TOOLBAR_HEIGHT: f32 = 40.0;

/// How the video area is presented right now. `Normal` keeps the full chrome;
/// `Immersive` hides toolbar + sidebar for edge-to-edge grid; `Spotlight` puts
/// a single camera full-bleed. None of these persist — they are session modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewFocus {
    Normal,
    Immersive,
    Spotlight(usize),
}

impl ViewFocus {
    pub fn is_normal(&self) -> bool {
        matches!(self, ViewFocus::Normal)
    }
}

/// After the reveal rail shows in immersive/spotlight, it auto-hides this long
/// once the pointer leaves the top edge.
pub(crate) const CHROME_REVEAL_SECS: u64 = 3;
/// Two clicks on the same cell within this window open its spotlight.
pub(crate) const DOUBLE_CLICK_MS: u128 = 350;

pub struct App {
    pub bridges: Vec<Arc<Mutex<GStreamerBridge>>>,
    pub videos: Vec<VideoWidget>,
    pub theme: Theme,
    pub sidebar: sidebar::Sidebar,
    pub window_size: iced::Size,
    pub layout_mode: LayoutMode,
    pub flex_main_idx: usize,
    /// Grid density / carousel / camera order. Persisted via
    /// `infrastructure::view_state`.
    pub view: ViewSettings,
    /// Zero-based page currently shown in grid mode. Not persisted — always
    /// starts at 0.
    pub current_page: usize,
    /// When the carousel last advanced the page.
    pub rotate_last_advance: Instant,
    /// The carousel stays paused until this instant after a manual page change
    /// or camera pick.
    pub interaction_pause_until: Option<Instant>,
    /// `[view] pause_hidden` — stop pipelines for cameras off the visible page.
    pub pause_hidden: bool,
    /// Spacing between staggered pipeline starts.
    pub stagger: Duration,
    /// Camera indices waiting for their pipeline to be started (initial launch
    /// and page flips both feed this).
    pub start_queue: VecDeque<usize>,
    /// Earliest instant the next queued camera may start.
    pub next_start_at: Instant,
    /// Whether each camera's video pipeline is currently running. Kept in
    /// lockstep with the other per-camera vecs.
    pub active_stream: Vec<bool>,
    /// When each camera's pipeline was last (re)started, used to show
    /// `CONNECTING` during the initial hand-shake. `None` once it has gone
    /// live or was never started.
    pub connecting_since: Vec<Option<Instant>>,
    pub is_recording: bool,
    pub audio_states: Vec<AudioState>,
    pub audio_pipelines: Vec<Rc<RefCell<Option<gstreamer::Pipeline>>>>,
    pub audio_urls: Vec<String>,
    pub audio_level_states: Vec<Arc<AudioLevelState>>,
    pub reconnect_states: Vec<ReconnectState>,
    pub backoff_states: Vec<BackoffState>,
    pub camera_configs: Vec<crate::config::CameraConfig>,
    pub is_fullscreen: bool,
    pub show_help: bool,
    /// Video presentation mode (Normal / Immersive / Spotlight). Not persisted.
    pub focus: ViewFocus,
    /// Reveal rail visible over the immersive/spotlight video.
    pub chrome_revealed: bool,
    pub chrome_revealed_at: Instant,
    /// Overflow (`⋯`) menu open in the toolbar.
    pub show_overflow_menu: bool,
    /// Last left-click on a grid cell, for double-click → spotlight detection.
    pub last_cell_click: Option<(usize, Instant)>,
    /// Grid cell the pointer is currently over (drives the on-cell action row).
    pub hovered_cell: Option<usize>,
    /// Last known pointer position in window coordinates — anchors the
    /// right-click command menu at the cursor.
    pub pointer_pos: iced::Point,
    /// Frame-tick counter, used to run slow metrics at a lower rate.
    pub tick_count: u64,
    pub toasts: Vec<Toast>,
    pub vu_peaks: Vec<f64>,
    pub vu_peak_since: Vec<Option<Instant>>,
    pub context_menu: Option<ContextMenu>,
    pub fps_history: Vec<Vec<f64>>,
    pub snapshot_config: SnapshotConfig,
    pub audio_config: AudioConfig,
    pub camera_enabled: Vec<bool>,
    /// True while the sidebar search box owns the keyboard, so single-key
    /// shortcuts must not steal the user's keystrokes.
    pub search_focused: bool,
    /// In-flight burst capture, advanced by the frame tick.
    pub pending_burst: Option<PendingBurst>,
}

/// A multi-frame snapshot burst in progress. Frames are captured one per
/// tick, which matches `snapshot::BURST_INTERVAL_MS`.
pub struct PendingBurst {
    pub camera_idx: usize,
    /// Shared timestamp so every frame in the burst lands in one series.
    pub timestamp: u64,
    pub next_seq: u32,
    pub remaining: u32,
    pub next_at: Instant,
}

#[allow(clippy::too_many_arguments)]
pub fn new_app(
    cameras: Vec<crate::config::CameraConfig>,
    recording_config: RecordingConfig,
    mut snapshot_config: SnapshotConfig,
    audio_config: AudioConfig,
    theme_name: String,
    logs_config: crate::config::LogsConfigFile,
    groups: Vec<crate::domain::groups::CameraGroup>,
    view_config: crate::config::ViewConfigFile,
) -> (App, Task<Message>) {
    let _ = gstreamer::init();

    if let Err(e) = crate::domain::snapshot::validate_config(&snapshot_config) {
        log::warn!("Invalid [snapshot] config ({e}); falling back to defaults");
        snapshot_config = SnapshotConfig::default();
    }

    let theme = match theme_name.to_lowercase().as_str() {
        "cosmic" => Theme::Cosmic,
        "dark" => Theme::Dark,
        "light" => Theme::Light,
        "amoled" => Theme::Amoled,
        "opencode" => Theme::OpenCode,
        _ => Theme::Cosmic,
    };
    let mut bridges = Vec::new();
    let mut videos = Vec::new();
    let mut camera_infos = Vec::new();
    let mut audio_pipelines = Vec::new();
    let mut audio_urls = Vec::new();
    let mut audio_level_states = Vec::new();
    let mut reconnect_states = Vec::new();
    let mut backoff_states = Vec::new();
    let mut vu_peaks = Vec::new();
    let mut vu_peak_since: Vec<Option<Instant>> = Vec::new();

    // Per-camera log directory (default: ~/logs/rust-rtsp-viewer).
    let log_dir = crate::infrastructure::recording_paths::ensure_log_dir(
        logs_config.dir.as_deref().unwrap_or(std::path::Path::new("~/logs/rust-rtsp-viewer")),
    )
    .unwrap_or_else(|_| std::path::PathBuf::from("logs"));
    let retention_days = logs_config.retention_days.unwrap_or(7);

    // Cameras whose bridge could not be constructed are dropped from every
    // parallel vec so the indices stay in lockstep; `camera_configs` is built
    // from this rather than the raw input for the same reason.
    let mut kept_cameras: Vec<crate::config::CameraConfig> = Vec::with_capacity(cameras.len());
    // Labels must be unique: two cameras with the same (or `safe_filename`-
    // colliding) label would share one log file and fight over rotation.
    let mut used_labels: std::collections::HashSet<String> = std::collections::HashSet::new();

    for cam in &cameras {
        let mut bridge = match GStreamerBridge::new(1920, 1080) {
            Ok(b) => b,
            Err(e) => {
                log::error!(
                    "Skipping camera {:?}: {e}",
                    cam.label
                        .as_deref()
                        .or(cam.name.as_deref())
                        .unwrap_or(cam.url.as_str())
                );
                continue;
            }
        };
        bridge.recording_config = recording_config.clone();
        let base_label = cam
            .label
            .clone()
            .or_else(|| cam.name.clone())
            .unwrap_or_else(|| format!("Camera {}", camera_infos.len() + 1));
        // Disambiguate on the `safe_filename` form (which is lossy), so
        // "Centro: A" and "Centro A" still get separate log files.
        let mut label = base_label.clone();
        let mut dup = 2;
        while !used_labels.insert(crate::infrastructure::recording_paths::safe_filename(&label)) {
            label = format!("{base_label} ({dup})");
            dup += 1;
        }
        let log_path = crate::infrastructure::recording_paths::log_path(&log_dir, &label);
        if let Ok(logger) = crate::infrastructure::recording_paths::CameraLogger::new(log_path, &label, retention_days) {
            bridge.set_logger(Arc::new(logger));
        }
        // Pipelines are NOT started here — a dozen HLS streams all connecting
        // inside `new_app` froze the window on launch. `update_frame` drains
        // `start_queue` one camera per `stagger`, and only for cameras that
        // should actually be visible.
        let bridge = Arc::new(Mutex::new(bridge));
        let video = VideoWidget::new(bridge.clone());
        kept_cameras.push(cam.clone());
        bridges.push(bridge);
        videos.push(video);
        audio_pipelines.push(Rc::new(RefCell::new(None)));
        audio_urls.push(cam.url.clone());
        audio_level_states.push(Arc::new(AudioLevelState::new()));
        reconnect_states.push(ReconnectState::new(15));
        backoff_states.push(BackoffState::new());
        vu_peaks.push(0.0);
        vu_peak_since.push(None);

        let kind = if cam.url.starts_with("http") || cam.url.starts_with("https") {
            "HLS"
        } else {
            "RTSP"
        };

        camera_infos.push(sidebar::CameraInfo {
            name: label,
            kind: kind.into(),
            status: sidebar::CameraStatus::Connecting,
            fps: 0.0,
            bitrate: "\u{2014}".into(),
            latency_ms: cam.latency_ms.unwrap_or(100) as u64,
            is_muted: true,
            is_recording: false,
            enabled: true,
            fps_history: Vec::new(),
        });
    }

    let count = bridges.len();

    // Groups reference camera indices; drop the whole set if any is invalid
    // rather than silently filtering to a broken view.
    let groups = match crate::domain::groups::validate_groups(&groups, count) {
        Ok(()) => groups,
        Err(e) => {
            log::warn!("Ignoring [[groups]]: {e}");
            Vec::new()
        }
    };
    let mut sidebar = sidebar::Sidebar::from_cameras(camera_infos);
    sidebar.groups = groups;

    // View settings: code defaults → `[view]` in config.toml → the persisted
    // `view.toml` (which wins). `pause_hidden` / `stagger_ms` stay
    // config-only — they are deployment knobs, not per-session tweaks.
    let mut view = view_config.into_settings(count);
    let pause_hidden = view_config.pause_hidden();
    let stagger = Duration::from_millis(view_config.stagger_ms());
    let mut layout_mode = if view_config.layout_is_flex() {
        LayoutMode::Flex
    } else {
        LayoutMode::Grid
    };

    let persisted = crate::infrastructure::view_state::load();
    if let Some(m) = persisted.mode.as_deref().and_then(crate::domain::view::GridMode::parse) {
        view.mode = m;
    }
    if let Some(r) = persisted.rotate_enabled {
        view.rotate_enabled = r;
    }
    if let Some(s) = persisted.rotate_secs {
        view.rotate_secs = s;
    }
    if let Some(o) = persisted.order {
        view.order = o;
    }
    match persisted.layout.as_deref() {
        Some("flex") => layout_mode = LayoutMode::Flex,
        Some("grid") => layout_mode = LayoutMode::Grid,
        _ => {}
    }
    if let Some(v) = persisted.sidebar_visible {
        sidebar.visible = v;
    }
    if let Some(g) = persisted.active_group
        && g < sidebar.groups.len()
    {
        sidebar.active_group = Some(g);
    }
    view.sanitize(count);
    sidebar.order = view.order.clone();

    // Start pipelines in display order so page 1 comes up first; the
    // `update_frame` drain skips any camera that should stay paused.
    let start_queue: VecDeque<usize> = crate::domain::view::apply_order(
        &view.order,
        &(0..count).collect::<Vec<_>>(),
    )
    .into();

    (
        App {
            bridges,
            videos,
            theme,
            sidebar,
            window_size: iced::Size::new(1280.0, 720.0),
            layout_mode,
            flex_main_idx: 0,
            view,
            current_page: 0,
            rotate_last_advance: Instant::now(),
            interaction_pause_until: None,
            pause_hidden,
            stagger,
            start_queue,
            next_start_at: Instant::now(),
            active_stream: vec![false; count],
            connecting_since: vec![None; count],
            is_recording: false,
            audio_states: vec![AudioState::Muted; count],
            audio_pipelines,
            audio_urls,
            audio_level_states,
            reconnect_states,
            backoff_states,
            camera_configs: kept_cameras,
            is_fullscreen: false,
            show_help: false,
            focus: ViewFocus::Normal,
            chrome_revealed: false,
            chrome_revealed_at: Instant::now(),
            show_overflow_menu: false,
            last_cell_click: None,
            hovered_cell: None,
            pointer_pos: iced::Point::new(200.0, 120.0),
            tick_count: 0,
            toasts: Vec::new(),
            vu_peaks: vec![0.0; count],
            vu_peak_since: vec![None; count],
            context_menu: None,
            fps_history: vec![Vec::new(); count],
            snapshot_config,
            audio_config,
            camera_enabled: vec![true; count],
            search_focused: false,
            pending_burst: None,
        },
        // iced 0.13's `window::Settings` has no "start maximized" flag, so ask
        // the compositor to maximize the window as soon as it exists. `size`
        // above stays as the restore size for when the user un-maximizes.
        iced::window::get_latest()
            .and_then(|id| iced::window::maximize(id, true)),
    )
}
