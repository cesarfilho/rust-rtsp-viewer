use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iced::Task;

use crate::domain::audio::{AudioConfig, AudioState};
use crate::domain::recording::RecordingConfig;
use crate::domain::snapshot::SnapshotConfig;
use crate::domain::view::ViewSettings;
use crate::infrastructure::audio::AudioLevelState;

use super::message::{LayoutMode, Message};
use super::sidebar;
use super::state::{ContextMenu, Toast};
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
    /// The connection to the rrv-daemon, and what the window shows of it.
    pub daemon: super::daemon::DaemonState,
    /// The connection thread; `None` in embedded mode.
    pub link: Option<crate::ipc::link::DaemonLink>,
    /// Requests sent to the daemon and not answered yet, by token.
    pub pending: std::collections::HashMap<u64, super::daemon::PendingRequest>,
    pub next_token: u64,
    /// A confirmation waiting for the user (modal for the keyboard).
    pub modal: Option<super::daemon::Modal>,
    /// The video engine: cameras, pipelines, reconnect, motion, notification state.
    pub engine: crate::engine::Engine,
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
    /// Camera briefly connected only to grab one preview frame for the flex
    /// thumbnail strip, and when it started.
    pub preview_cam: Option<(usize, Instant)>,
    /// Cameras whose preview attempt is over (frame grabbed or timed out).
    pub preview_done: Vec<bool>,
    pub is_recording: bool,
    pub audio_states: Vec<AudioState>,
    pub audio_pipelines: Vec<Rc<RefCell<Option<gstreamer::Pipeline>>>>,
    pub audio_urls: Vec<String>,
    pub audio_level_states: Vec<Arc<AudioLevelState>>,
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
    /// True while the sidebar search box owns the keyboard, so single-key
    /// shortcuts must not steal the user's keystrokes.
    pub search_focused: bool,
    /// In-flight burst capture, advanced by the frame tick.
    pub pending_burst: Option<PendingBurst>,
    /// Everything persisted to `zones.toml`, so saving one camera keeps the rest.
    pub zones_file: crate::infrastructure::zone_state::ZonesFile,
    /// Zone editor session, `Some` while the user is drawing.
    pub zone_edit: Option<ZoneEdit>,
    /// The recordings view (timeline + player), open while `Some`.
    pub recordings: Option<super::recordings::RecordingsView>,
    /// Where this window reads the daemon's recordings (`[recording] dir`).
    pub recordings_dir: std::path::PathBuf,
    /// Test aid (`RRV_OPEN_RECORDINGS=1`): open the recordings view as soon as the
    /// daemon connects, so it can be checked without synthetic key presses.
    pub open_recordings_on_connect: bool,
    /// `RRV_OPEN_RECORDINGS=play`: also play the latest finished segment (test aid).
    pub recordings_autoplay: bool,
    /// `RRV_OPEN_RECORDINGS=compare`: with autoplay, put every other camera side by side too.
    pub recordings_compare: bool,
}

/// A zone being drawn on one camera. Vertices are only committed to
/// `App.zones` when the user finishes the polygon.
pub struct ZoneEdit {
    pub camera_idx: usize,
    pub temp_vertices: Vec<crate::domain::zones::Point>,
    /// A save is waiting for the daemon's answer.
    pub saving: bool,
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
    notify_config: crate::domain::notify::NotifyConfig,
    motion_config: crate::domain::motion::MotionConfig,
    daemon_options: super::DaemonOptions,
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
    // The engine builds one bridge per camera (and drops any that cannot be
    // built); everything the window needs per camera is derived from it so the
    // indices stay in lockstep. Pipelines are NOT started here — `update_frame`
    // drains the engine's start queue one camera per `stagger`.
    let zones_file = crate::infrastructure::zone_state::load();
    let mut engine = crate::engine::Engine::new(crate::engine::EngineSettings {
        cameras: &cameras,
        recording: &recording_config,
        motion: motion_config,
        notify: notify_config,
        logs: &logs_config,
        pause_hidden: view_config.pause_hidden(),
        stagger: Duration::from_millis(view_config.stagger_ms()),
        zones: &zones_file,
    });
    let count = engine.camera_count();

    // The daemon, if there is one. With a daemon this window only *shows*: it
    // must not also record, detect or notify (two owners would do it all twice).
    let socket = daemon_options
        .socket
        .clone()
        .unwrap_or_else(crate::ipc::default_socket_path);
    let mode = super::daemon::initial_mode(daemon_options.embedded, socket.exists());
    let (daemon, link) = if mode == super::daemon::Mode::Embedded {
        (super::daemon::DaemonState::embedded(socket), None)
    } else {
        log::info!("rrv-daemon found at {}: connecting", socket.display());
        let link = crate::ipc::link::DaemonLink::spawn(socket.clone());
        (super::daemon::DaemonState::connecting(socket), Some(link))
    };
    engine.set_display_only(daemon.is_daemon_mode());

    let videos: Vec<VideoWidget> = engine
        .bridges
        .iter()
        .map(|b| VideoWidget::new(b.clone()))
        .collect();
    let audio_pipelines: Vec<_> = (0..count).map(|_| Rc::new(RefCell::new(None))).collect();
    let audio_urls: Vec<String> = engine
        .camera_configs
        .iter()
        .map(|c| c.url.clone())
        .collect();
    let audio_level_states: Vec<_> = (0..count)
        .map(|_| Arc::new(AudioLevelState::new()))
        .collect();
    let camera_infos: Vec<sidebar::CameraInfo> = engine
        .names
        .iter()
        .zip(&engine.camera_configs)
        .map(|(name, cam)| sidebar::CameraInfo {
            name: name.clone(),
            kind: if cam.url.starts_with("http") {
                "HLS"
            } else {
                "RTSP"
            }
            .into(),
            status: sidebar::CameraStatus::Connecting,
            fps: 0.0,
            bitrate: "\u{2014}".into(),
            latency_ms: cam.latency_ms.unwrap_or(100) as u64,
            is_muted: true,
            is_recording: false,
            enabled: true,
            fps_history: Vec::new(),
        })
        .collect();

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
    let mut layout_mode = if view_config.layout_is_flex() {
        LayoutMode::Flex
    } else {
        LayoutMode::Grid
    };

    let persisted = crate::infrastructure::view_state::load();
    if let Some(m) = persisted
        .mode
        .as_deref()
        .and_then(crate::domain::view::GridMode::parse)
    {
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
    engine.set_start_order(&crate::domain::view::apply_order(
        &view.order,
        &(0..count).collect::<Vec<_>>(),
    ));

    (
        App {
            daemon,
            link,
            pending: std::collections::HashMap::new(),
            next_token: 1,
            modal: None,
            engine,
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
            preview_cam: None,
            preview_done: vec![false; count],
            is_recording: false,
            audio_states: vec![AudioState::Muted; count],
            audio_pipelines,
            audio_urls,
            audio_level_states,
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
            search_focused: false,
            pending_burst: None,
            zones_file,
            zone_edit: None,
            recordings: None,
            recordings_dir: recording_config.dir.clone(),
            open_recordings_on_connect: std::env::var_os("RRV_OPEN_RECORDINGS").is_some(),
            recordings_autoplay: std::env::var("RRV_OPEN_RECORDINGS")
                .is_ok_and(|v| v == "play" || v == "compare"),
            recordings_compare: std::env::var("RRV_OPEN_RECORDINGS").is_ok_and(|v| v == "compare"),
        },
        // iced 0.13's `window::Settings` has no "start maximized" flag, so ask
        // the compositor to maximize the window as soon as it exists. `size`
        // above stays as the restore size for when the user un-maximizes.
        iced::window::latest().and_then(|id| iced::window::maximize(id, true)),
    )
}
