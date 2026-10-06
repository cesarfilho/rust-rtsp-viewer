use log::warn;
use serde::Deserialize;

use crate::domain::audio::AudioConfig;
use crate::domain::recording::{Container, RecordingConfig};
use crate::domain::snapshot::SnapshotConfig;
use crate::domain::view::{self, GridMode, ViewSettings};

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub latency_ms: Option<u32>,
    /// GStreamer decoder element name. Default: `decodebin` (more stable).
    pub decoder: Option<String>,
    /// Ask the camera to retransmit lost packets. Default: `true`.
    pub do_retransmission: Option<bool>,
    /// Snapshot configuration (F12 / `s` key).
    pub snapshot: Option<SnapshotConfigFile>,
    /// Recording configuration (`r` key).
    pub recording: Option<RecordingConfigFile>,
    /// Audio configuration (`m` key toggles mute).
    pub audio: Option<AudioConfigFile>,
    /// Multi-camera grid mode.
    pub cameras: Option<Vec<CameraConfig>>,
    /// UI theme: "cosmic" (default), "dark", "light", "amoled" or "opencode".
    pub theme: Option<String>,
    /// Interface language: `"pt-BR"` or `"en"`. Default: Portuguese (the system `LANG` is ignored).
    /// The choice made in the window's menu is saved in `view.toml` and wins.
    pub language: Option<String>,
    /// Per-camera log file directory. Default: `~/logs/rust-rtsp-viewer`.
    /// Each camera gets its own file: `<dir>/<safe_label>.log`.
    pub logs: Option<LogsConfigFile>,
    /// Named camera groups for sidebar/grid filtering. `[[groups]]` with
    /// `name` and `cameras = [0, 2, 3]` (0-based indices into `[[cameras]]`).
    pub groups: Option<Vec<crate::domain::groups::CameraGroupFile>>,
    /// `[view]` — how the grid is presented: density, pagination, carousel,
    /// lazy decoding, staggered startup.
    pub view: Option<ViewConfigFile>,
    /// `[notifications]` — desktop alerts for motion and offline cameras.
    pub notifications: Option<NotificationsConfigFile>,
    /// `[webhook]` — the daemon posts its notifications to a URL (works with the
    /// window closed).
    pub webhook: Option<crate::webhook::WebhookFile>,
    /// `[motion]` — frame-difference detector tuning (`enabled`, `threshold`,
    /// `contour_area`, `sample_stride`).
    pub motion: Option<crate::domain::motion::MotionConfigFile>,
    /// `[daemon]` — the network channel between the window and the daemon (they may be on different
    /// machines): the daemon `listen`s on a TCP address, the window connects to an `address`, both
    /// sharing a `token`.
    pub daemon: Option<DaemonFile>,
    /// `[mqtt]` — the daemon publishes camera state and events to an MQTT broker (Home Assistant).
    pub mqtt: Option<crate::mqtt::MqttFile>,
    /// `[detect]` — object detection (YOLO) on the frames where motion is seen.
    pub detect: Option<crate::domain::detect::DetectFile>,
    /// `[retention]` — how long the daemon keeps recordings (`motion_days`,
    /// `manual_days`, `max_disk_percent`).
    pub retention: Option<crate::domain::retention::RetentionFile>,
}

/// `[daemon]`: the channel over the network (TCP + token; the traffic is **not encrypted**).
#[derive(Deserialize, Clone, Default)]
pub struct DaemonFile {
    /// Daemon side: where to listen for windows, e.g. `"0.0.0.0:7878"` (all interfaces) or
    /// `"192.168.1.10:7878"` (one). Requires `token`. Absent = the local socket only.
    pub listen: Option<String>,
    /// Window side: the daemon to connect to, `"tcp://192.168.1.10:7878"` (or a socket path). The
    /// `--daemon` flag wins. Absent = the local socket.
    pub address: Option<String>,
    /// The shared secret (16+ characters, e.g. `openssl rand -hex 24`); accepts `${NAME}`. The window also
    /// reads `RRV_TOKEN` or the keyring secret `rrv_token`.
    pub token: Option<String>,
}

// The token is a secret: a stray `{:?}` of the config must never print it.
impl std::fmt::Debug for DaemonFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaemonFile")
            .field("listen", &self.listen)
            .field("address", &self.address)
            .field("token", &self.token.as_ref().map(|_| "****"))
            .finish()
    }
}

/// Flat mirror of the `[notifications]` section.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct NotificationsConfigFile {
    /// Send desktop notifications (needs `notify-send`). Default: false.
    pub enabled: Option<bool>,
    /// Minimum seconds between notifications of one kind per camera (>= 5). Default: 60.
    pub cooldown_secs: Option<u64>,
}

impl NotificationsConfigFile {
    pub fn into_config(self) -> crate::domain::notify::NotifyConfig {
        let mut c = crate::domain::notify::NotifyConfig::default();
        if let Some(e) = self.enabled {
            c.enabled = e;
        }
        if let Some(s) = self.cooldown_secs {
            c.cooldown_secs = s.max(crate::domain::notify::MIN_COOLDOWN_SECS);
        }
        c
    }
}

/// Flat mirror of the `[view]` section. These are the *initial* values; the
/// user's runtime tweaks are persisted separately in
/// `infrastructure::view_state` and layered on top at startup.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct ViewConfigFile {
    /// Grid density: `"auto"` (default — every camera on one balanced page) or
    /// a fixed preset like `"2x2"`, `"3x3"`, `"4x4"`. A fixed preset is what
    /// turns on pagination.
    pub mode: Option<String>,
    /// Carousel dwell time in seconds (clamped 3..=300). Default: 10.
    pub rotate_secs: Option<u64>,
    /// Start with the page carousel running. Default: false.
    pub rotate_enabled: Option<bool>,
    /// Only decode the cameras on the visible page (plus the next page as a
    /// prefetch); pause the rest. Default: true. Set false to keep every
    /// pipeline running so page flips are instant.
    pub pause_hidden: Option<bool>,
    /// Milliseconds between camera pipeline starts at launch / on page flips,
    /// so a dozen streams don't all connect at once. Default: 200. `0` starts
    /// them as fast as the frame tick allows.
    pub stagger_ms: Option<u64>,
    /// Initial layout: `"grid"` (default) or `"flex"`.
    pub layout: Option<String>,
}

/// Milliseconds between staggered pipeline starts when `[view] stagger_ms` is
/// unset.
pub const DEFAULT_STAGGER_MS: u64 = 200;

impl ViewConfigFile {
    /// Build the runtime [`ViewSettings`] for `n_cameras`, clamping the
    /// carousel interval and defaulting `order` to the natural `0..n`.
    pub fn into_settings(&self, n_cameras: usize) -> ViewSettings {
        let mode = match self.mode.as_deref() {
            None => GridMode::Auto,
            Some(s) => GridMode::parse(s).unwrap_or_else(|| {
                warn!("Ignoring [view] mode = {s:?}: expected \"auto\" or e.g. \"3x3\"");
                GridMode::Auto
            }),
        };
        let mut settings = ViewSettings {
            mode,
            rotate_enabled: self.rotate_enabled.unwrap_or(false),
            rotate_secs: self.rotate_secs.unwrap_or(view::ROTATE_DEFAULT_SECS),
            order: (0..n_cameras).collect(),
        };
        settings.sanitize(n_cameras);
        settings
    }

    /// Whether hidden-camera pipelines should be paused. Default: `true`.
    pub fn pause_hidden(&self) -> bool {
        self.pause_hidden.unwrap_or(true)
    }

    /// Staggered-start spacing. Default: [`DEFAULT_STAGGER_MS`].
    pub fn stagger_ms(&self) -> u64 {
        self.stagger_ms.unwrap_or(DEFAULT_STAGGER_MS)
    }

    /// `true` when `[view] layout = "flex"`.
    pub fn layout_is_flex(&self) -> bool {
        self.layout
            .as_deref()
            .map(|s| s.trim().eq_ignore_ascii_case("flex"))
            .unwrap_or(false)
    }
}

/// One entry in the `[[cameras]]` TOML array.
/// Each field overrides the global default for
/// that camera only.
#[derive(Debug, Deserialize, Clone)]
pub struct CameraConfig {
    /// RTSP stream URL (required).
    pub url: String,
    /// Short CLI alias for this camera (optional). When set, the user can open
    /// this camera with `rust-rtsp-viewer <name>` instead of typing the URL.
    /// Matched case-insensitively. See `resolve_camera_alias`.
    pub name: Option<String>,
    /// Human-readable label shown in the sidebar
    /// header (optional, defaults to the host).
    pub label: Option<String>,
    /// Network latency in ms (overrides global
    /// `latency_ms`; default 100). Maps directly to `rtspsrc latency` —
    /// the only stream buffering knob; everything else is left to GStreamer.
    pub latency_ms: Option<u32>,
    /// GStreamer decoder element (overrides global
    /// `decoder`; default `decodebin`).
    pub decoder: Option<String>,
    /// Audio volume when this camera is selected in
    /// the grid (0.0–1.0). When set, clicking the
    /// cell starts audio playback for this camera and
    /// mutes all others. When absent, clicking only
    /// highlights the cell visually (no audio).
    pub audio_volume: Option<f32>,
    /// Override the source element for this camera.
    /// `true` → `uridecodebin` (required for HLS/HTTP streams).
    /// `false` → `rtspsrc` (RTSP-only).
    /// When absent, auto-detected from the URL scheme:
    /// `http://` / `https://` → `true`; `rtsp://` → `false`.
    pub use_uridecodebin: Option<bool>,
    /// Ask the camera to retransmit lost packets. Overrides the global
    /// `do_retransmission` for this camera only. When absent, the global
    /// value is used (which defaults to `true` when not set in config.toml).
    pub do_retransmission: Option<bool>,
    /// Lower-resolution stream for the grid tiles. When set, the camera
    /// decodes `sub_url` while it is a tile and switches to `url` (main) for
    /// the spotlight, the flex main view, and while recording.
    pub sub_url: Option<String>,
}

/// Flat mirror of `domain::snapshot::SnapshotConfig`
/// for TOML deserialisation. All fields are `Option`
/// so the user can override only the ones they care
/// about; missing fields fall back to the
/// `SnapshotConfig::default()` values.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct SnapshotConfigFile {
    /// Output directory. Default:
    /// `$XDG_PICTURES_DIR/rust-rtsp-viewer` or
    /// `~/Pictures/rust-rtsp-viewer`.
    pub dir: Option<std::path::PathBuf>,
    /// PNG quality, 0–100. Default: 92.
    /// (PNG is lossless in practice — the
    /// `quality` field controls zlib compression
    /// level via `image::codecs::png::CompressionType`.)
    pub quality: Option<u8>,
    /// Number of frames per burst. 1 = single shot.
    /// 2+ = captures `burst_count` frames at
    /// `BURST_INTERVAL_MS` intervals (100ms). Default: 1.
    pub burst_count: Option<u32>,
}

/// Flat mirror of `domain::recording::RecordingConfig`
/// for TOML deserialisation. All fields are `Option`
/// so the user can override only the ones they care
/// about; missing fields fall back to the
/// `RecordingConfig::default()` values.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct RecordingConfigFile {
    /// Output directory. Default:
    /// `$XDG_VIDEOS_DIR/rust-rtsp-viewer` or
    /// `~/Videos/rust-rtsp-viewer`.
    pub dir: Option<std::path::PathBuf>,
    /// Maximum segment duration in seconds. Range
    /// 10..=86400. Default: 600 (10 min).
    pub max_segment_duration_secs: Option<u32>,
    /// Maximum segment size in bytes. Range
    /// 1 MiB..=16 GiB. Default: 1 GiB.
    pub max_segment_size_bytes: Option<u64>,
    /// Container: "mkv" (default) or "mp4".
    pub container: Option<String>,
    /// Record automatically while motion is detected. Default: false.
    pub on_motion: Option<bool>,
    /// Seconds to keep recording after the last motion (3..=3600). Default: 15.
    pub motion_post_roll_secs: Option<u32>,
    /// Seconds of video to keep before the motion (0..=30; 0 = off). Default: 5.
    pub motion_pre_roll_secs: Option<u32>,
    /// Record the camera's audio too (RTSP cameras that have an audio track). Default: false.
    pub record_audio: Option<bool>,
}

/// Flat mirror of `domain::audio::AudioConfig`
/// for TOML deserialisation. All fields are
/// `Option` so the user can override only the
/// ones they care about; missing fields fall
/// back to the `AudioConfig::default()` values.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct AudioConfigFile {
    /// Whether the audio branch is enabled.
    /// Default: `true`. Setting this to
    /// `false` in `config.toml` is equivalent
    /// to passing `--no-audio` on the CLI.
    pub enabled: Option<bool>,
    /// Initial volume, `0.0..=1.0`.
    /// Default: 0.8. Out-of-range values are
    /// rejected by `main.rs` (filter to
    /// finite values in 0..=1).
    pub volume: Option<f32>,
}

impl RecordingConfigFile {
    pub fn into_config(self) -> RecordingConfig {
        let mut config = RecordingConfig::default();
        if let Some(dir) = self.dir {
            config.dir = dir;
        }
        if let Some(d) = self.max_segment_duration_secs {
            config.max_segment_duration_secs = d;
        }
        if let Some(s) = self.max_segment_size_bytes {
            config.max_segment_size_bytes = s;
        }
        if let Some(m) = self.on_motion {
            config.on_motion = m;
        }
        if let Some(p) = self.motion_post_roll_secs {
            config.motion_post_roll_secs = p.clamp(3, 3600);
        }
        if let Some(a) = self.record_audio {
            config.record_audio = a;
        }
        if let Some(p) = self.motion_pre_roll_secs {
            config.motion_pre_roll_secs = p.min(30);
        }
        if let Some(c) = self.container {
            config.container = match c.to_lowercase().as_str() {
                "mp4" => Container::Mp4,
                _ => Container::Mkv,
            };
        }
        config
    }
}

impl AudioConfigFile {
    pub fn into_config(self) -> AudioConfig {
        let mut config = AudioConfig::default();
        if let Some(enabled) = self.enabled {
            config.enabled = enabled;
        }
        if let Some(v) = self.volume {
            // Out-of-range or non-finite values keep the default rather than
            // silently producing a silent (or clipping) stream.
            if v.is_finite() && (0.0..=1.0).contains(&v) {
                config.volume = v;
            } else {
                warn!("Ignoring [audio] volume = {v}: must be between 0.0 and 1.0");
            }
        }
        config
    }
}

/// Flat mirror for TOML deserialisation of the `[logs]` section.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct LogsConfigFile {
    /// Output directory for per-camera log files.
    /// Default: `~/logs/rust-rtsp-viewer`.
    pub dir: Option<std::path::PathBuf>,
    /// Number of days to keep rotated log files before pruning.
    /// Default: 7 (one week).
    pub retention_days: Option<u32>,
}

impl SnapshotConfigFile {
    pub fn into_config(self) -> SnapshotConfig {
        let mut config = SnapshotConfig::default();
        if let Some(dir) = self.dir {
            config.dir = dir;
        }
        // Clamp to the ranges `snapshot::validate_config` enforces (quality
        // 1..=100, burst 1..=50). An unclamped `burst_count` overflows the
        // `seq + remaining - 1` arithmetic in `advance_burst` and would run a
        // 1080p PNG encode every tick for billions of frames.
        if let Some(q) = self.quality {
            config.quality = q.clamp(1, 100);
        }
        if let Some(b) = self.burst_count {
            config.burst_count = b.clamp(1, 50);
        }
        config
    }
}

/// Resolve a CLI token (e.g. `cam1`, `garagem`, `2`) to the index of a camera
/// in `[[cameras]]`. Resolution order:
///   1. `name` — exact match, case-insensitive (after trim).
///   2. `label` — case-insensitive, ignoring spaces (so `riojardim` matches
///      "Rio Jardim").
///   3. ordinal — `camN` (with or without the `cam` prefix) or a bare `N`,
///      1-based, mapped to index `N-1`.
///
/// Returns `None` when nothing matches or the ordinal is out of range.
pub fn resolve_camera_alias(token: &str, cameras: &[CameraConfig]) -> Option<usize> {
    let t = token.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }

    // 1. Explicit name (exact, case-insensitive).
    if let Some(idx) = cameras.iter().position(|c| {
        c.name
            .as_deref()
            .map(|n| n.trim().to_lowercase() == t)
            .unwrap_or(false)
    }) {
        return Some(idx);
    }

    // 2. Label (case-insensitive, spaces removed on both sides).
    let t_nospace: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if let Some(idx) = cameras.iter().position(|c| {
        c.label
            .as_deref()
            .map(|l| {
                l.trim()
                    .to_lowercase()
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .collect::<String>()
                    == t_nospace
            })
            .unwrap_or(false)
    }) {
        return Some(idx);
    }

    // 3. Ordinal: `camN` or bare `N`, 1-based.
    let digits = t.strip_prefix("cam").unwrap_or(&t);
    if let Ok(n) = digits.parse::<usize>()
        && n >= 1
        && n <= cameras.len()
    {
        return Some(n - 1);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_into_config_clamps_out_of_range_values() {
        let cfg = SnapshotConfigFile {
            dir: None,
            quality: Some(0),
            burst_count: Some(9999),
        }
        .into_config();
        assert_eq!(cfg.quality, 1, "quality 0 is rejected by validate_config");
        assert_eq!(cfg.burst_count, 50, "burst_count is capped at 50");
    }

    #[test]
    fn snapshot_into_config_keeps_valid_values() {
        let cfg = SnapshotConfigFile {
            dir: None,
            quality: Some(75),
            burst_count: Some(10),
        }
        .into_config();
        assert_eq!(cfg.quality, 75);
        assert_eq!(cfg.burst_count, 10);
    }

    fn cam(name: Option<&str>, label: Option<&str>) -> CameraConfig {
        CameraConfig {
            url: "rtsp://x/stream".to_string(),
            name: name.map(|s| s.to_string()),
            label: label.map(|s| s.to_string()),
            latency_ms: None,
            decoder: None,
            audio_volume: None,
            use_uridecodebin: None,
            do_retransmission: None,
            sub_url: None,
        }
    }

    fn fixture() -> Vec<CameraConfig> {
        vec![
            cam(Some("frente"), Some("Garagem")),
            cam(None, Some("Rio Jardim Sofia")),
            cam(None, None),
        ]
    }

    #[test]
    fn matches_explicit_name_case_insensitive() {
        assert_eq!(resolve_camera_alias("Frente", &fixture()), Some(0));
        assert_eq!(resolve_camera_alias("  frente ", &fixture()), Some(0));
    }

    #[test]
    fn matches_label_ignoring_case_and_spaces() {
        assert_eq!(resolve_camera_alias("garagem", &fixture()), Some(0));
        assert_eq!(resolve_camera_alias("riojardimsofia", &fixture()), Some(1));
        assert_eq!(
            resolve_camera_alias("Rio Jardim Sofia", &fixture()),
            Some(1)
        );
    }

    #[test]
    fn matches_ordinal_camn_and_bare_number() {
        assert_eq!(resolve_camera_alias("cam1", &fixture()), Some(0));
        assert_eq!(resolve_camera_alias("cam3", &fixture()), Some(2));
        assert_eq!(resolve_camera_alias("2", &fixture()), Some(1));
    }

    #[test]
    fn name_wins_over_label_and_ordinal() {
        // A camera named "cam2" must resolve to itself, not the 2nd ordinal.
        let cams = vec![cam(None, Some("First")), cam(Some("cam2"), Some("Second"))];
        // "cam2" matches the explicit name on index 1 (also the ordinal here),
        // but to prove precedence, name it on index 0:
        let cams2 = vec![cam(Some("cam2"), Some("First")), cam(None, Some("Second"))];
        assert_eq!(resolve_camera_alias("cam2", &cams), Some(1));
        assert_eq!(resolve_camera_alias("cam2", &cams2), Some(0));
    }

    #[test]
    fn view_config_defaults_when_section_absent() {
        let v = ViewConfigFile::default().into_settings(11);
        assert_eq!(v.mode, GridMode::Auto);
        assert!(!v.rotate_enabled);
        assert_eq!(v.rotate_secs, crate::domain::view::ROTATE_DEFAULT_SECS);
        assert_eq!(v.order, (0..11).collect::<Vec<_>>());
        assert!(ViewConfigFile::default().pause_hidden());
        assert_eq!(ViewConfigFile::default().stagger_ms(), DEFAULT_STAGGER_MS);
        assert!(!ViewConfigFile::default().layout_is_flex());
    }

    #[test]
    fn view_config_parses_mode_and_clamps_interval() {
        let f = ViewConfigFile {
            mode: Some("3x3".into()),
            rotate_secs: Some(99999),
            rotate_enabled: Some(true),
            pause_hidden: Some(false),
            stagger_ms: Some(0),
            layout: Some("FLEX".into()),
        };
        let v = f.into_settings(5);
        assert_eq!(v.mode, GridMode::Fixed { cols: 3, rows: 3 });
        assert_eq!(v.rotate_secs, crate::domain::view::ROTATE_MAX_SECS);
        assert!(v.rotate_enabled);
        assert!(!f.pause_hidden());
        assert_eq!(f.stagger_ms(), 0);
        assert!(f.layout_is_flex());
    }

    #[test]
    fn view_config_unknown_mode_falls_back_to_auto() {
        let f = ViewConfigFile {
            mode: Some("banana".into()),
            ..Default::default()
        };
        assert_eq!(f.into_settings(4).mode, GridMode::Auto);
    }

    #[test]
    fn unknown_token_and_out_of_range_ordinal_return_none() {
        assert_eq!(resolve_camera_alias("inexistente", &fixture()), None);
        assert_eq!(resolve_camera_alias("cam9", &fixture()), None);
        assert_eq!(resolve_camera_alias("0", &fixture()), None);
        assert_eq!(resolve_camera_alias("", &fixture()), None);
    }

    #[test]
    fn parses_motion_notifications_and_auto_recording_sections() {
        let cfg: Config = toml::from_str(
            r#"
            [motion]
            threshold = 40
            contour_area = 0.02
            sample_stride = 0

            [notifications]
            enabled = true
            cooldown_secs = 1

            [recording]
            on_motion = true
            motion_post_roll_secs = 1

            [[cameras]]
            url = "rtsp://host/stream"
            "#,
        )
        .expect("config should parse");

        let motion = cfg.motion.expect("[motion] must be read").into_config();
        assert_eq!(motion.threshold, 40);
        assert!((motion.contour_area - 0.02).abs() < 1e-9);
        assert_eq!(
            motion.sample_stride, 1,
            "stride 0 would never advance the scan"
        );

        let notify = cfg.notifications.expect("[notifications]").into_config();
        assert!(notify.enabled);
        assert_eq!(notify.cooldown_secs, 5, "cooldown has a floor");

        let rec = cfg.recording.expect("[recording]").into_config();
        assert!(rec.on_motion);
        assert_eq!(rec.motion_post_roll_secs, 3, "post-roll has a floor");
    }

    #[test]
    fn omitted_motion_section_keeps_detector_defaults() {
        let cfg: Config = toml::from_str("[[cameras]]\nurl = \"rtsp://h/s\"\n").unwrap();
        assert!(cfg.motion.is_none());
        let d = crate::domain::motion::MotionConfig::default();
        assert!(d.enabled && d.sample_stride >= 1);
    }

    /// The annotated reference file is documentation users copy from; it must
    /// keep parsing as the schema evolves.
    #[test]
    fn example_config_parses() {
        let cfg: Config = toml::from_str(include_str!("../../../config.toml.example"))
            .expect("config.toml.example must stay valid");
        assert!(cfg.cameras.is_some_and(|c| !c.is_empty()));
    }
}
