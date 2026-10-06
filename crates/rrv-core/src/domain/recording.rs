//! Recording domain — pure types, state machine, and helpers.
//!
//! Item 9 of the 14-item feature plan. The recording
//! infrastructure (splitmuxsink / filesink wiring) lives in
//! `crate::infrastructure::recording`; this module owns
//! the *semantics* of "is the user currently recording?"
//! independently of any GStreamer knowledge.
//!
//! State machine:
//! ```text
//!        ┌─────── start() ───────┐
//!        │                       ▼
//!   ┌────────┐               ┌───────────┐
//!   │  Idle  │ ── toggle() ─▶│ Recording │
//!   └────────┘ ◀── toggle() ─ └───────────┘
//!        ▲                       │
//!        └──────── stop() ───────┘
//! ```
//!
//! Invalid transitions (double-start, stop-while-idle)
//! return `RecordingError::InvalidState` and leave the
//! state machine untouched.

// Many of this module's public items
// (validate, is_recording, start, stop, toggle,
// format_elapsed, ToggleOutcome, RecordingState::Recording,
// RecordingError, RecordingIoError) are exercised by
// the unit tests in this file and consumed by
// `crate::infrastructure::recording`. The dead_code
// lint would otherwise fire on items that are part
// of the public API but not yet used in the GTK
// wiring (item 9 final wiring, which is done in the
// same commit). Silencing at the module level is
// the cleanest way to keep the warnings down.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use thiserror::Error;

/// Maximum segment duration. Once a segment reaches
/// this length, the muxer rolls a new file (item 9
/// uses matroskamux, which handles this natively
/// inside splitmuxsink). Default: 10 minutes — long
/// enough to keep file count manageable, short enough
/// that a corrupted segment loses at most 10 min.
pub const DEFAULT_MAX_SEGMENT_DURATION_SECS: u32 = 600;

/// Maximum segment size. Once a segment reaches this
/// many bytes, the muxer rolls a new file. Default:
/// 1 GiB. Pairs with `max_segment_duration_secs` —
/// whichever is hit first triggers the split.
pub const DEFAULT_MAX_SEGMENT_SIZE_BYTES: u64 = 1024 * 1024 * 1024;

/// Output container / muxer. The two options supported
/// out of the box; anything else requires additional
/// `gstreamer1.0-plugins-*` system deps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Container {
    /// Matroska / WebM (.mkv). The most resilient
    /// container for surveillance: handles arbitrary
    /// codec combinations, supports mid-stream codec
    /// reconfiguration, and recovers from arbitrary
    /// truncation. Default.
    #[default]
    Mkv,
    /// MP4 (.mp4). Better tooling support (ffmpeg,
    /// QuickTime, mobile) but requires the muxer to
    /// see a clean EOS for the file to be readable.
    /// A power loss mid-recording produces an
    /// unrecoverable file. NOT recommended for
    /// surveillance use cases.
    Mp4,
}

impl Container {
    /// File extension (without the dot).
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mkv => "mkv",
            Self::Mp4 => "mp4",
        }
    }

    /// The GStreamer muxer element name.
    /// `matroskamux` for `Mkv`, `mp4mux` for `Mp4`.
    /// `mp4mux` requires the camera stream to have
    /// timing information; `matroskamux` does not.
    pub fn muxer_element(self) -> &'static str {
        match self {
            Self::Mkv => "matroskamux",
            Self::Mp4 => "mp4mux",
        }
    }

    /// Display label for the sidebar.
    pub fn label(self) -> &'static str {
        match self {
            Self::Mkv => "MKV (Matroska)",
            Self::Mp4 => "MP4",
        }
    }
}

/// User-facing configuration for recording. Resolved
/// from CLI / `config.toml` / defaults in `main.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RecordingConfig {
    /// Output directory. Created on first recording
    /// start if it doesn't exist.
    pub dir: PathBuf,
    /// Maximum segment duration. Range: 10s ..= 24h.
    pub max_segment_duration_secs: u32,
    /// Maximum segment size. Range: 1 MiB ..= 16 GiB.
    pub max_segment_size_bytes: u64,
    /// Container / muxer.
    pub container: Container,
    /// Record automatically while motion is detected.
    pub on_motion: bool,
    /// Keep recording this long after the last motion. Range: 3s ..= 1h.
    pub motion_post_roll_secs: u32,
    /// Seconds of video kept **before** the motion that starts a recording, taken
    /// from a ring of the camera's own encoded stream (RTSP H.264/H.265 only).
    /// 0 turns it off. Range: 0 ..= 30. Rounded up to whole GOPs.
    pub motion_pre_roll_secs: u32,
    /// Put the camera's audio track in the recording (RTSP, AAC/G.711...). Off by
    /// default: recording people's voices is a decision, not a side effect.
    pub record_audio: bool,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            dir: default_videos_dir(),
            max_segment_duration_secs: DEFAULT_MAX_SEGMENT_DURATION_SECS,
            max_segment_size_bytes: DEFAULT_MAX_SEGMENT_SIZE_BYTES,
            container: Container::default(),
            on_motion: false,
            motion_post_roll_secs: DEFAULT_MOTION_POST_ROLL_SECS,
            motion_pre_roll_secs: DEFAULT_MOTION_PRE_ROLL_SECS,
            record_audio: false,
        }
    }
}

pub const DEFAULT_MOTION_POST_ROLL_SECS: u32 = 15;
pub const DEFAULT_MOTION_PRE_ROLL_SECS: u32 = 5;

/// What the motion-triggered recorder should do this sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionRecAction {
    None,
    Start,
    Stop,
}

/// Decide whether to start or stop an event recording.
///
/// * Motion while idle → start.
/// * Only a recording *we* started (`auto_started`) is ever stopped, and only
///   once `secs_since_motion` reaches the post-roll — a manual recording is
///   never cut short by a quiet scene.
pub fn motion_recording_action(
    is_recording: bool,
    auto_started: bool,
    motion_active: bool,
    secs_since_motion: u64,
    post_roll_secs: u32,
) -> MotionRecAction {
    if motion_active && !is_recording {
        MotionRecAction::Start
    } else if is_recording
        && auto_started
        && !motion_active
        && secs_since_motion >= u64::from(post_roll_secs)
    {
        MotionRecAction::Stop
    } else {
        MotionRecAction::None
    }
}

/// Errors that can be produced by the recording
/// domain. `Io` wraps a `std::io::Error` (file ops);
/// `InvalidState` covers state-machine violations
/// (double-start, stop-while-idle); `Config` is for
/// `validate_config` rejections.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecordingError {
    #[error("recording config error: {0}")]
    Config(String),
    #[error("invalid recording state transition: {0}")]
    InvalidState(&'static str),
    #[error("I/O error during recording: {0}")]
    Io(RecordingIoError),
}

/// Wrapper for I/O errors. We can't use `std::io::Error`
/// directly in `#[derive(PartialEq, Eq)]` (it doesn't
/// implement them), so we keep a `String` form. This
/// also keeps tests easy — `RecordingError::Io("...")`
/// compares structurally.
#[derive(Debug, Error, PartialEq, Eq)]
#[error("{0}")]
pub struct RecordingIoError(pub String);

impl From<std::io::Error> for RecordingError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(RecordingIoError(e.to_string()))
    }
}

impl RecordingConfig {
    /// Validate the config. Returns `Ok(())` if the
    /// config is usable, `RecordingError::Config` with
    /// a human-readable message otherwise. Cheap to
    /// call (no I/O) — directory existence is checked
    /// at `RecordingSession::start` time.
    pub fn validate(&self) -> Result<(), RecordingError> {
        if self.dir.as_os_str().is_empty() {
            return Err(RecordingError::Config("output directory is empty".into()));
        }
        if self.max_segment_duration_secs < 10 {
            return Err(RecordingError::Config(format!(
                "max_segment_duration_secs must be >= 10 (got {})",
                self.max_segment_duration_secs
            )));
        }
        if self.max_segment_duration_secs > 86_400 {
            return Err(RecordingError::Config(format!(
                "max_segment_duration_secs must be <= 86400 (got {})",
                self.max_segment_duration_secs
            )));
        }
        const ONE_MIB: u64 = 1024 * 1024;
        const SIXTEEN_GIB: u64 = 16 * ONE_MIB * 1024;
        if self.max_segment_size_bytes < ONE_MIB {
            return Err(RecordingError::Config(format!(
                "max_segment_size_bytes must be >= 1 MiB (got {})",
                self.max_segment_size_bytes
            )));
        }
        if self.max_segment_size_bytes > SIXTEEN_GIB {
            return Err(RecordingError::Config(format!(
                "max_segment_size_bytes must be <= 16 GiB (got {})",
                self.max_segment_size_bytes
            )));
        }
        Ok(())
    }
}

/// Recording state machine. Owns the current
/// recording state and the time at which the current
/// recording started (so the UI can show elapsed
/// time).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecordingState {
    /// Not recording. The `r` key transitions to
    /// `Recording`.
    #[default]
    Idle,
    /// Currently recording. `started_at_unix_secs`
    /// is the `SystemTime::now()` at `start()` time,
    /// in UNIX seconds — so the elapsed-time formatter
    /// doesn't need a clock parameter.
    Recording {
        path: PathBuf,
        started_at_unix_secs: u64,
        segment_index: u32,
    },
}

impl RecordingState {
    /// True if the state machine is currently in
    /// `Recording`. The `is_recording()` getter on
    /// `RecordingSession` is a thin wrapper around
    /// this.
    pub fn is_recording(&self) -> bool {
        matches!(self, Self::Recording { .. })
    }

    /// Transition to `Recording` if currently `Idle`.
    /// Returns the new state on success, or
    /// `RecordingError::InvalidState` if a recording
    /// is already in progress.
    pub fn start(&self, path: PathBuf, now_unix_secs: u64) -> Result<Self, RecordingError> {
        match self {
            Self::Idle => Ok(Self::Recording {
                path,
                started_at_unix_secs: now_unix_secs,
                segment_index: 0,
            }),
            Self::Recording { .. } => Err(RecordingError::InvalidState("already recording")),
        }
    }

    /// Transition to `Idle` if currently `Recording`.
    /// Returns the previous recording path so the
    /// caller can log it / show it in a notification.
    /// Returns `RecordingError::InvalidState` if not
    /// recording.
    pub fn stop(&self) -> Result<(Self, PathBuf, u64, u32), RecordingError> {
        match self {
            Self::Recording {
                path,
                started_at_unix_secs,
                segment_index,
            } => Ok((
                Self::Idle,
                path.clone(),
                *started_at_unix_secs,
                *segment_index,
            )),
            Self::Idle => Err(RecordingError::InvalidState("not recording")),
        }
    }

    /// Toggle: start if idle, stop if recording.
    /// `start_path` is the new file path used when
    /// transitioning to `Recording`. Returns the new
    /// state and, if we just stopped, the path we
    /// were recording to (so the caller can show a
    /// notification).
    pub fn toggle(
        &self,
        start_path: PathBuf,
        now_unix_secs: u64,
    ) -> Result<ToggleOutcome, RecordingError> {
        match self {
            Self::Idle => {
                let next = self.start(start_path, now_unix_secs)?;
                Ok(ToggleOutcome::Started(next))
            }
            Self::Recording { .. } => {
                let (next, path, started_at, seg_idx) = self.stop()?;
                Ok(ToggleOutcome::Stopped {
                    new_state: next,
                    finalised_path: path,
                    started_at_unix_secs: started_at,
                    segment_index: seg_idx,
                })
            }
        }
    }
}

/// Result of a `RecordingState::toggle()` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToggleOutcome {
    Started(RecordingState),
    Stopped {
        new_state: RecordingState,
        finalised_path: PathBuf,
        started_at_unix_secs: u64,
        segment_index: u32,
    },
}

impl ToggleOutcome {
    /// The new state after the toggle. Always
    /// available, regardless of which variant.
    pub fn new_state(&self) -> &RecordingState {
        match self {
            Self::Started(s) => s,
            Self::Stopped { new_state, .. } => new_state,
        }
    }
}

/// Compute the elapsed seconds of the given
/// `Recording` variant, given a "now" UNIX
/// timestamp. Saturates to 0 for `Idle` (caller can
/// treat this as "0 elapsed, no recording").
pub fn elapsed_secs(state: &RecordingState, now_unix_secs: u64) -> u64 {
    match state {
        RecordingState::Idle => 0,
        RecordingState::Recording {
            started_at_unix_secs,
            ..
        } => now_unix_secs.saturating_sub(*started_at_unix_secs),
    }
}

/// Format a duration in seconds as `HH:MM:SS`.
/// Always positive. Saturates at 99:59:59 for
/// absurdly large values (would require running
/// for > 100 hours which is unrealistic but the
/// formatter handles it gracefully).
pub fn format_elapsed(secs: u64) -> String {
    let h = (secs / 3600).min(99);
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

/// Humanize a byte count for the sidebar. Uses
/// binary prefixes (KiB / MiB / GiB) — disk
/// manufacturers love to lie with decimal
/// prefixes, and we want to be honest with the
/// user about what they're filling up.
pub fn humanize_bytes_binary(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Generate a recording-segment filename.
/// Format: `rust-rtsp-viewer-YYYY-MM-DD-HHMMSS-NNN.{ext}`.
/// `sequence` is the segment index (000, 001, ...).
/// Uses the same Hinnant `civil_from_days` algorithm
/// as `snapshot::generate_filename` (single source
/// of truth would be cleaner — TODO if a 3rd caller
/// appears).
pub fn generate_filename(timestamp_unix_secs: u64, sequence: u32, container: Container) -> String {
    let (y, mo, d, h, mi, s) = unix_secs_to_ymdhms(timestamp_unix_secs);
    format!(
        "rust-rtsp-viewer-{:04}-{:02}-{:02}-{:02}{:02}{:02}-{:03}.{}",
        y,
        mo,
        d,
        h,
        mi,
        s,
        sequence,
        container.extension()
    )
}

/// Decompose UNIX seconds into `(year, month, day, hour,
/// minute, second)`. UTC. Uses Howard Hinnant's
/// `civil_from_days` algorithm — the same one
/// `domain::snapshot` uses, duplicated here rather
/// than shared because a 3rd caller would justify
/// the extraction. See `domain::snapshot` for the
/// full derivation.
fn unix_secs_to_ymdhms(unix_secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let days = (unix_secs / 86_400) as i64;
    let secs_of_day = (unix_secs % 86_400) as u32;
    let h = secs_of_day / 3600;
    let m = (secs_of_day % 3600) / 60;
    let s = secs_of_day % 60;
    let (y, mo, d) = civil_from_days(days);
    (y, mo, d, h, m, s)
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Compute segment timestamps for a planned
/// recording. `start_unix_secs` is the first segment;
/// `interval_secs` is the gap between segments
/// (equal to `max_segment_duration_secs`); `count`
/// is how many segments to plan. Used by tests /
/// status preview only — the actual muxer handles
/// its own segment timestamps at runtime.
pub fn compute_segment_timestamps(
    start_unix_secs: u64,
    interval_secs: u32,
    count: u32,
) -> Vec<u64> {
    (0..count)
        .map(|i| start_unix_secs + (i as u64) * (interval_secs as u64))
        .collect()
}

/// Render the user-facing status text for the
/// sidebar. Returns:
/// - `Idle` → `"○ idle"` (light-circle, dim)
/// - `Recording { ... }` → `"● REC 00:01:23  segment 000"` (red dot)
pub fn render_status(state: &RecordingState, now_unix_secs: u64) -> String {
    match state {
        RecordingState::Idle => "○ idle".to_string(),
        RecordingState::Recording { segment_index, .. } => {
            format!(
                "● REC  {}  segment {:03}",
                format_elapsed(elapsed_secs(state, now_unix_secs)),
                segment_index
            )
        }
    }
}

/// Default video directory. Mirrors
/// `domain::snapshot::default_pictures_dir` — looks
/// at `$XDG_VIDEOS_DIR` first, then falls back to
/// `~/Videos`. The `rust-rtsp-viewer` subdirectory
/// is appended here so recordings don't pollute the
/// user's Videos directory.
pub fn default_videos_dir() -> PathBuf {
    let base = if let Ok(p) = std::env::var("XDG_VIDEOS_DIR") {
        if !p.is_empty() {
            PathBuf::from(p)
        } else {
            PathBuf::from("./Videos")
        }
    } else if let Some(home) = std::env::var_os("HOME") {
        let mut p = PathBuf::from(home);
        p.push("Videos");
        p
    } else {
        PathBuf::from("./Videos")
    };
    base.join("rust-rtsp-viewer")
}

/// The current UNIX time in seconds. Wrapped
/// because `SystemTime` operations on a `0`
/// (pre-epoch) system would panic.
pub fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Container ---

    #[test]
    fn motion_starts_recording_when_idle() {
        assert_eq!(
            motion_recording_action(false, false, true, 0, 15),
            MotionRecAction::Start
        );
    }

    #[test]
    fn motion_stops_only_after_post_roll() {
        assert_eq!(
            motion_recording_action(true, true, false, 14, 15),
            MotionRecAction::None
        );
        assert_eq!(
            motion_recording_action(true, true, false, 15, 15),
            MotionRecAction::Stop
        );
    }

    #[test]
    fn motion_never_stops_a_manual_recording() {
        assert_eq!(
            motion_recording_action(true, false, false, 999, 15),
            MotionRecAction::None
        );
    }

    #[test]
    fn motion_keeps_recording_while_active() {
        assert_eq!(
            motion_recording_action(true, true, true, 999, 15),
            MotionRecAction::None
        );
    }

    #[test]
    fn container_default_is_mkv() {
        assert_eq!(Container::default(), Container::Mkv);
    }

    #[test]
    fn container_extensions() {
        assert_eq!(Container::Mkv.extension(), "mkv");
        assert_eq!(Container::Mp4.extension(), "mp4");
    }

    #[test]
    fn container_muxer_elements() {
        assert_eq!(Container::Mkv.muxer_element(), "matroskamux");
        assert_eq!(Container::Mp4.muxer_element(), "mp4mux");
    }

    // --- RecordingConfig::default ---

    #[test]
    fn default_config_has_sensible_values() {
        let cfg = RecordingConfig::default();
        assert!(!cfg.dir.as_os_str().is_empty());
        assert!(cfg.max_segment_duration_secs >= 60);
        assert!(cfg.max_segment_size_bytes >= 1024 * 1024);
        assert_eq!(cfg.container, Container::Mkv);
    }

    // --- validate ---

    #[test]
    fn validate_rejects_empty_dir() {
        let cfg = RecordingConfig {
            dir: PathBuf::new(),
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, RecordingError::Config(_)));
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn validate_rejects_tiny_segment_duration() {
        let cfg = RecordingConfig {
            max_segment_duration_secs: 5,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, RecordingError::Config(_)));
        assert!(err.to_string().contains("duration"));
    }

    #[test]
    fn validate_rejects_huge_segment_duration() {
        let cfg = RecordingConfig {
            max_segment_duration_secs: 86_401,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, RecordingError::Config(_)));
    }

    #[test]
    fn validate_rejects_tiny_segment_size() {
        let cfg = RecordingConfig {
            max_segment_size_bytes: 1024,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, RecordingError::Config(_)));
        assert!(err.to_string().contains("size"));
    }

    #[test]
    fn validate_rejects_huge_segment_size() {
        let cfg = RecordingConfig {
            max_segment_size_bytes: 32 * 1024 * 1024 * 1024,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, RecordingError::Config(_)));
    }

    #[test]
    fn validate_accepts_default_config() {
        assert!(RecordingConfig::default().validate().is_ok());
    }

    // --- RecordingState transitions ---

    #[test]
    fn idle_is_not_recording() {
        assert!(!RecordingState::Idle.is_recording());
    }

    #[test]
    fn recording_is_recording() {
        let s = RecordingState::Recording {
            path: PathBuf::from("/tmp/x.mkv"),
            started_at_unix_secs: 0,
            segment_index: 0,
        };
        assert!(s.is_recording());
    }

    #[test]
    fn start_from_idle_succeeds() {
        let next = RecordingState::Idle
            .start(PathBuf::from("/tmp/r.mkv"), 1_700_000_000)
            .unwrap();
        match next {
            RecordingState::Recording {
                path,
                started_at_unix_secs,
                segment_index,
            } => {
                assert_eq!(path, PathBuf::from("/tmp/r.mkv"));
                assert_eq!(started_at_unix_secs, 1_700_000_000);
                assert_eq!(segment_index, 0);
            }
            _ => panic!("expected Recording, got {next:?}"),
        }
    }

    #[test]
    fn start_from_recording_fails() {
        let s = RecordingState::Recording {
            path: PathBuf::from("/tmp/r.mkv"),
            started_at_unix_secs: 0,
            segment_index: 0,
        };
        let err = s.start(PathBuf::from("/tmp/r2.mkv"), 0).unwrap_err();
        assert_eq!(err, RecordingError::InvalidState("already recording"));
    }

    #[test]
    fn stop_from_idle_fails() {
        let err = RecordingState::Idle.stop().unwrap_err();
        assert_eq!(err, RecordingError::InvalidState("not recording"));
    }

    #[test]
    fn stop_from_recording_returns_idle_and_path() {
        let s = RecordingState::Recording {
            path: PathBuf::from("/tmp/r.mkv"),
            started_at_unix_secs: 1_000,
            segment_index: 5,
        };
        let (next, path, started, seg) = s.stop().unwrap();
        assert_eq!(next, RecordingState::Idle);
        assert_eq!(path, PathBuf::from("/tmp/r.mkv"));
        assert_eq!(started, 1_000);
        assert_eq!(seg, 5);
    }

    #[test]
    fn toggle_from_idle_starts() {
        let outcome = RecordingState::Idle
            .toggle(PathBuf::from("/tmp/r.mkv"), 5_000)
            .unwrap();
        match outcome {
            ToggleOutcome::Started(s) => {
                assert!(s.is_recording());
            }
            _ => panic!("expected Started, got {outcome:?}"),
        }
    }

    #[test]
    fn toggle_from_recording_stops() {
        let s = RecordingState::Recording {
            path: PathBuf::from("/tmp/r.mkv"),
            started_at_unix_secs: 0,
            segment_index: 0,
        };
        let outcome = s.toggle(PathBuf::from("/tmp/x"), 0).unwrap();
        match outcome {
            ToggleOutcome::Stopped {
                new_state,
                finalised_path,
                ..
            } => {
                assert_eq!(new_state, RecordingState::Idle);
                assert_eq!(finalised_path, PathBuf::from("/tmp/r.mkv"));
            }
            _ => panic!("expected Stopped, got {outcome:?}"),
        }
    }

    // --- elapsed / format ---

    #[test]
    fn elapsed_for_idle_is_zero() {
        assert_eq!(elapsed_secs(&RecordingState::Idle, 1_000_000), 0);
    }

    #[test]
    fn elapsed_for_recording_subtracts_started_at() {
        let s = RecordingState::Recording {
            path: PathBuf::from("/tmp/r.mkv"),
            started_at_unix_secs: 100,
            segment_index: 0,
        };
        assert_eq!(elapsed_secs(&s, 250), 150);
    }

    #[test]
    fn elapsed_saturates_at_zero_when_clock_goes_back() {
        let s = RecordingState::Recording {
            path: PathBuf::from("/tmp/r.mkv"),
            started_at_unix_secs: 1_000,
            segment_index: 0,
        };
        // Clock skew: now < started_at. Saturating
        // subtraction keeps us at 0 rather than
        // wrapping into a huge u64.
        assert_eq!(elapsed_secs(&s, 500), 0);
    }

    #[test]
    fn format_elapsed_pads_correctly() {
        assert_eq!(format_elapsed(0), "00:00:00");
        assert_eq!(format_elapsed(7), "00:00:07");
        assert_eq!(format_elapsed(65), "00:01:05");
        assert_eq!(format_elapsed(3_725), "01:02:05");
        // Saturation: anything > 99 hours clamps.
        // 100 hours = 360_000s → 99:00:00 (h clamped,
        // m/s carried over).
        assert_eq!(format_elapsed(360_000), "99:00:00");
    }

    // --- format_elapsed specific cases ---

    #[test]
    fn test_format_duration_seconds() {
        assert_eq!(format_elapsed(45), "00:00:45");
    }

    #[test]
    fn test_format_duration_minutes() {
        assert_eq!(format_elapsed(125), "00:02:05");
    }

    #[test]
    fn test_format_duration_hours() {
        assert_eq!(format_elapsed(3661), "01:01:01");
    }

    // --- RecordingConfig defaults ---

    #[test]
    fn test_recording_config_defaults() {
        let cfg = RecordingConfig::default();
        assert_eq!(
            cfg.max_segment_duration_secs,
            DEFAULT_MAX_SEGMENT_DURATION_SECS
        );
        assert_eq!(cfg.max_segment_size_bytes, DEFAULT_MAX_SEGMENT_SIZE_BYTES);
        assert_eq!(cfg.container, Container::Mkv);
        assert!(!cfg.dir.as_os_str().is_empty());
    }

    // --- humanize_bytes_binary ---

    #[test]
    fn humanize_bytes_binary_units() {
        assert_eq!(humanize_bytes_binary(0), "0 B");
        assert_eq!(humanize_bytes_binary(512), "512 B");
        assert_eq!(humanize_bytes_binary(1024), "1.0 KiB");
        assert_eq!(humanize_bytes_binary(1536), "1.5 KiB");
        assert_eq!(humanize_bytes_binary(1024 * 1024), "1.0 MiB");
        assert_eq!(humanize_bytes_binary(1024 * 1024 * 1024), "1.0 GiB");
    }

    // --- generate_filename ---

    #[test]
    fn generate_filename_mkv() {
        // 2024-01-15 13:30:05 UTC = 1_705_325_405
        let name = generate_filename(1_705_325_405, 0, Container::Mkv);
        assert_eq!(name, "rust-rtsp-viewer-2024-01-15-133005-000.mkv");
    }

    #[test]
    fn generate_filename_mp4_with_nonzero_sequence() {
        let name = generate_filename(1_705_325_405, 42, Container::Mp4);
        assert_eq!(name, "rust-rtsp-viewer-2024-01-15-133005-042.mp4");
    }

    #[test]
    fn generate_filename_epoch() {
        // 1970-01-01 00:00:00 UTC
        let name = generate_filename(0, 0, Container::Mkv);
        assert_eq!(name, "rust-rtsp-viewer-1970-01-01-000000-000.mkv");
    }

    // --- compute_segment_timestamps ---

    #[test]
    fn compute_segment_timestamps_basic() {
        let ts = compute_segment_timestamps(1_000, 600, 3);
        assert_eq!(ts, vec![1_000, 1_600, 2_200]);
    }

    #[test]
    fn compute_segment_timestamps_zero_count() {
        let ts = compute_segment_timestamps(1_000, 600, 0);
        assert!(ts.is_empty());
    }

    // --- render_status ---

    #[test]
    fn render_status_idle() {
        assert_eq!(render_status(&RecordingState::Idle, 0), "○ idle");
    }

    #[test]
    fn render_status_recording() {
        let s = RecordingState::Recording {
            path: PathBuf::from("/tmp/r.mkv"),
            started_at_unix_secs: 0,
            segment_index: 3,
        };
        // now = 65s after start → 00:01:05
        assert_eq!(render_status(&s, 65), "● REC  00:01:05  segment 003");
    }
}
