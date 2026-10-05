use crate::domain::diagnostics::Hint;
use crate::domain::groups::CameraGroup;
use crate::domain::metrics::PacketStats;
use crate::domain::timeline::EventTimeline;

/// How many events the sidebar timeline keeps before dropping the oldest.
pub const TIMELINE_MAX_EVENTS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarView {
    Cameras,
    Info,
    Diagnostics,
    Timeline,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CameraStatus {
    Live,
    Offline,
    Reconnecting,
    Recording,
    Disabled,
    /// Pipeline is starting up (initial staggered launch, or coming back onto
    /// the visible page) but has not produced a frame yet. Distinct from
    /// `Offline`, which means the stream actually failed.
    Connecting,
    /// Deliberately stopped because the camera is off the visible page and
    /// `[view] pause_hidden` is on. Costs no CPU; not a fault.
    Paused,
}

impl CameraStatus {
    pub fn label(&self) -> &'static str {
        match self {
            CameraStatus::Live => "LIVE",
            CameraStatus::Offline => "OFFLINE",
            CameraStatus::Reconnecting => "RECONNECTING",
            CameraStatus::Recording => "RECORDING",
            CameraStatus::Disabled => "DESATIVADA",
            CameraStatus::Connecting => "CONNECTING",
            CameraStatus::Paused => "PAUSED",
        }
    }

    /// Sentence-case Portuguese label for the UI (the all-caps `label` reads as
    /// shouting).
    pub fn label_pt(&self) -> &'static str {
        match self {
            CameraStatus::Live => "Ao vivo",
            CameraStatus::Offline => "Offline",
            CameraStatus::Reconnecting => "Reconectando",
            CameraStatus::Recording => "Gravando",
            CameraStatus::Disabled => "Desativada",
            CameraStatus::Connecting => "Conectando\u{2026}",
            CameraStatus::Paused => "Pausada",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CameraInfo {
    pub name: String,
    pub kind: String,
    pub status: CameraStatus,
    pub fps: f64,
    pub bitrate: String,
    pub latency_ms: u64,
    pub is_muted: bool,
    pub is_recording: bool,
    pub enabled: bool,
    /// Short tail of recent fps samples for the row sparkline (cap ~16).
    pub fps_history: Vec<f64>,
}

#[derive(Clone, Debug)]
pub enum Message {
    TabClicked(SidebarView),
    CameraClicked(usize),
    SearchChanged(String),
    CameraToggled(usize, bool),
    /// The user finished editing the search box (Enter).
    SearchSubmitted,
    /// Filter the camera list / grid to a named group. `None` = show all.
    GroupSelected(Option<usize>),
    /// Move a camera one slot earlier / later in the display order (the ▲ / ▼
    /// buttons on each camera row).
    CameraMovedUp(usize),
    CameraMovedDown(usize),
    /// Pointer entered (`Some`) or left (`None`) a camera row — reveals that
    /// row's reorder / menu controls.
    HoverRow(Option<usize>),
    /// The row `⋯` button — open the shared command menu for that camera.
    ShowRowMenu(usize),
    /// Expand / collapse the Inspector's "Avançado" block.
    ToggleInfoAdvanced,
    /// A timeline row was clicked.
    EventClicked(usize),
}

#[derive(Clone, Debug, Default)]
pub struct MetricsSnapshot {
    pub fps: f64,
    pub latency_ms: Option<u64>,
    pub jitter_ms: Option<u64>,
    pub decode_time_ms: Option<u64>,
    pub packet_stats: PacketStats,
    pub dropped: u64,
    pub decode_errors: u64,
    pub reconnects: u32,
    pub last_reconnect_ms: Option<u64>,
    pub frames: u64,
    pub bytes: u64,
    pub uptime_secs: u64,
    pub codec: Option<String>,
    /// Factory name of the decoder in use (`avdec_h264`, `nvh264dec`, …).
    pub decoder: Option<String>,
    pub decoder_hw: bool,
    /// `Main` / `Sub` — only set for cameras that have a `sub_url`.
    pub stream_quality: Option<&'static str>,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub framerate_num: Option<i32>,
    pub framerate_den: Option<i32>,
    pub avg_luma: Option<u8>,
    pub luma_stddev: Option<u8>,
    pub static_secs: Option<u64>,
    pub last_error: Option<String>,
    pub is_live: bool,
    pub is_recording: bool,
    pub recording_elapsed_secs: u64,
    pub audio_level: f64,
    pub vu_peak: f64,
    pub fps_history: Vec<f64>,
}

#[derive(Clone)]
pub struct Sidebar {
    pub active_view: SidebarView,
    pub cameras: Vec<CameraInfo>,
    pub selected: Option<usize>,
    pub search_query: String,
    pub visible: bool,
    pub diagnostics: Vec<Hint>,
    pub selected_metrics: Option<MetricsSnapshot>,
    /// Recording/connectivity/snapshot events, newest last.
    pub timeline: EventTimeline,
    /// Named camera groups from `[[groups]]`. Empty = grouping disabled.
    pub groups: Vec<CameraGroup>,
    /// Index into `groups` of the active filter, or `None` for "all cameras".
    pub active_group: Option<usize>,
    /// Camera display order — a permutation of `0..cameras.len()`, mirrored
    /// from `App.view.order` so the camera list and grid agree. Empty until
    /// `new_app` fills it (treated as natural order while empty).
    pub order: Vec<usize>,
    /// Camera-list row the pointer is currently over.
    pub hover_row: Option<usize>,
    /// Whether the Inspector's "Avançado" block is expanded.
    pub info_advanced: bool,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self::new()
    }
}

impl Sidebar {
    pub fn new() -> Self {
        Self::from_cameras(vec![])
    }

    /// Camera indices visible under the active group filter. With no active
    /// group (or no groups configured) this is every camera, `0..total`.
    pub fn visible_camera_indices(&self, total: usize) -> Vec<usize> {
        match self.active_group.and_then(|g| self.groups.get(g)) {
            Some(grp) => grp
                .camera_indices
                .iter()
                .copied()
                .filter(|&i| i < total)
                .collect(),
            None => (0..total).collect(),
        }
    }

    pub fn from_cameras(cameras: Vec<CameraInfo>) -> Self {
        let cameras_len = cameras.len();
        Self {
            active_view: SidebarView::Cameras,
            cameras,
            selected: None,
            search_query: String::new(),
            visible: true,
            diagnostics: vec![],
            selected_metrics: None,
            timeline: EventTimeline::new(TIMELINE_MAX_EVENTS),
            groups: vec![],
            active_group: None,
            order: (0..cameras_len).collect(),
            hover_row: None,
            info_advanced: false,
        }
    }

    /// Display order, tolerant of an unset (`new`) sidebar: falls back to the
    /// natural `0..cameras.len()` when `order` is empty or stale.
    pub fn display_order(&self) -> Vec<usize> {
        crate::domain::view::apply_order(
            &self.order,
            &(0..self.cameras.len()).collect::<Vec<_>>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidebar_default_view() {
        let sidebar = Sidebar::new();
        assert_eq!(sidebar.active_view, SidebarView::Cameras);
    }

    #[test]
    fn sidebar_view_equality() {
        assert_eq!(SidebarView::Cameras, SidebarView::Cameras);
        assert_ne!(SidebarView::Cameras, SidebarView::Info);
    }

    #[test]
    fn sidebar_message_debug() {
        let msg = Message::CameraClicked(0);
        assert!(format!("{:?}", msg).contains("CameraClicked"));
    }

    #[test]
    fn camera_status_label() {
        assert_eq!(CameraStatus::Live.label(), "LIVE");
        assert_eq!(CameraStatus::Offline.label(), "OFFLINE");
        assert_eq!(CameraStatus::Reconnecting.label(), "RECONNECTING");
        assert_eq!(CameraStatus::Recording.label(), "RECORDING");
        assert_eq!(CameraStatus::Disabled.label(), "DESATIVADA");
    }

    #[test]
    fn metrics_snapshot_default_recording_fields() {
        let m = MetricsSnapshot::default();
        assert!(!m.is_recording);
        assert_eq!(m.recording_elapsed_secs, 0);
    }

    #[test]
    fn visible_indices_follow_active_group() {
        let mut s = Sidebar::new();
        assert_eq!(s.visible_camera_indices(4), vec![0, 1, 2, 3]);

        s.groups = vec![CameraGroup::new("Front", vec![1, 3])];
        // No active group → still all cameras.
        assert_eq!(s.visible_camera_indices(4), vec![0, 1, 2, 3]);

        s.active_group = Some(0);
        assert_eq!(s.visible_camera_indices(4), vec![1, 3]);
        // Out-of-range indices are dropped, not panicked on.
        assert_eq!(s.visible_camera_indices(2), vec![1]);
    }
}
