//! Snapshot domain — pure data + planning.
//!
//! Splitting the snapshot feature into a pure domain layer
//! (`SnapshotConfig`, filename generation, validation,
//! notification text) and an infrastructure layer
//! (`infrastructure::snapshot_writer` — GStreamer + GTK)
//! keeps the side-effecting capture path small and makes
//! the user-visible strings (filenames, toast messages)
//! unit-testable without a GTK context.
//!
//! No `gst::Pipeline`, no `gtk::Widget`, no `std::fs::write`
//! in this module — everything is pure data transformation
//! and `Result` returning.

#![allow(dead_code)]

use std::fmt;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Where to write snapshots, how good they should look,
/// and how many frames to capture in a burst.
///
/// All fields are independent — there's no hidden coupling
/// (e.g. `quality = 0` is just a *value*; whether it's
/// valid is `validate_config`'s job). This makes the struct
/// easy to build from a `config.toml` table and easy to
/// debug-print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotConfig {
    /// Directory the snapshot is written into. The
    /// directory is created at save-time (not at config-
    /// load time) so a missing path doesn't abort startup.
    pub dir: PathBuf,
    /// JPEG/PNG quality 1-100. The encoder maps this
    /// 1:1 to libjpeg's quality scale. `100` = lossless
    /// (or near-lossless, depending on codec). 80 is the
    /// "visually lossless" sweet spot.
    pub quality: u8,
    /// How many frames to capture in a single burst
    /// (Ctrl+F12). `1` is the normal single-shot mode
    /// (F12); `5` is the default burst. The interval
    /// between frames is fixed at `BURST_INTERVAL_MS` so
    /// bursts are predictable.
    pub burst_count: u32,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        // Default directory follows the XDG Base Directory
        // spec ($XDG_PICTURES_DIR or `$HOME/Pictures`),
        // matching what the `directories` crate would pick
        // — we just compute it ourselves to keep the dep
        // count down.
        let dir = default_pictures_dir().join("rust-rtsp-viewer");
        Self {
            dir,
            quality: 85,
            burst_count: 1,
        }
    }
}

impl fmt::Display for SnapshotConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SnapshotConfig {{ dir={:?}, quality={}, burst={} }}",
            self.dir, self.quality, self.burst_count
        )
    }
}

/// Things that can go wrong before we even touch the
/// encoder. Encoder/IO errors live on the infrastructure
/// side; this enum is the configuration-time surface.
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("invalid quality {0} (must be 1-100)")]
    InvalidQuality(u8),
    #[error("invalid burst_count {0} (must be 1-50)")]
    InvalidBurstCount(u32),
    #[error("invalid snapshot dir {path:?}: empty path")]
    InvalidDir { path: PathBuf },
    #[error("directory {path:?} could not be created: {source}")]
    DirectoryCreation {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Validate a `SnapshotConfig`. Returns the first error
/// found (cheap; we don't need to surface all of them at
/// once). Pure — does not touch the filesystem; the
/// directory existence check is a separate step
/// (`ensure_dir_exists` in the infrastructure layer).
pub fn validate_config(cfg: &SnapshotConfig) -> Result<(), SnapshotError> {
    if cfg.quality == 0 {
        return Err(SnapshotError::InvalidQuality(cfg.quality));
    }
    if cfg.burst_count == 0 || cfg.burst_count > 50 {
        return Err(SnapshotError::InvalidBurstCount(cfg.burst_count));
    }
    if cfg.dir.as_os_str().is_empty() {
        return Err(SnapshotError::InvalidDir {
            path: cfg.dir.clone(),
        });
    }
    Ok(())
}

/// Format a snapshot filename as
/// `rust-rtsp-viewer-YYYY-MM-DD-HHMMSS-NNN.png`.
///
/// `sequence` is the within-burst index (`1` for single
/// shots, `1..=N` for bursts). The wall-clock timestamp
/// comes from the *caller* (so the function is pure and
/// deterministic in tests).
///
/// Examples (UTC):
/// - (1749310212, 1) → "rust-rtsp-viewer-2025-06-07-143012-001.png"
/// - (1749310212, 5) → "rust-rtsp-viewer-2025-06-07-143012-005.png"
///
/// The timestamp is decomposed in **UTC**, on purpose: this function is pure
/// and its tests assert exact strings, filenames then sort in wall-clock order
/// for any single user, and the toast/timeline show the local time so no
/// information is lost. (The per-camera log files use local time — that split
/// is deliberate and documented in `AGENTS.md`.)
pub fn generate_filename(timestamp_unix_secs: u64, sequence: u32) -> String {
    let (year, month, day, hour, min, sec) = unix_secs_to_ymdhms(timestamp_unix_secs);
    format!(
        "rust-rtsp-viewer-{year:04}-{month:02}-{day:02}-{hour:02}{min:02}{sec:02}-{sequence:03}.png"
    )
}

/// Compute the offset (in ms) at which each burst frame
/// should be captured, relative to the burst start
/// (frame 0). Frame 0 is at t=0; subsequent frames are
/// `BURST_INTERVAL_MS` apart.
///
/// Example: `compute_burst_timestamps(100, 5) == [0, 100, 200, 300, 400]`.
pub fn compute_burst_timestamps(interval_ms: u64, count: u32) -> Vec<u64> {
    (0..count).map(|i| i as u64 * interval_ms).collect()
}

/// Fixed interval between burst frames. 100ms gives 10
/// fps, which is fast enough to feel "instant" and slow
/// enough to actually see different poses in a scene
/// (a person walking, a car entering the frame).
pub const BURST_INTERVAL_MS: u64 = 100;

/// Render the toast/notification text shown in the
/// overlay after a successful snapshot.
///
/// Format:
/// ```text
/// ✓ Snapshot saved: rust-rtsp-viewer-...png (142.3 KB)
///   /home/user/Pictures/rust-rtsp-viewer/
/// ```
///
/// Two lines: the headline (filename + size) and the
/// directory (so the user knows where to find it). The
/// directory is rendered as a separate line because file
/// managers often show just the directory on hover.
pub fn render_notification(filename: &str, size_bytes: u64, dir: &Path) -> String {
    let size = humanize_bytes(size_bytes);
    format!("✓ Snapshot saved: {filename} ({size})\n  {}", dir.display())
}

/// Format a byte count as a human-readable string.
///
/// - < 1024 → "B" (raw bytes)
/// - < 1024² → "KB" (one decimal)
/// - < 1024³ → "MB" (one decimal)
/// - else → "GB" (one decimal)
///
/// We avoid `format!`-ing a `f64` into a `&'static str`
/// (impossible), so this returns a `String` — the caller
/// owns the allocation.
pub fn humanize_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes < KB {
        format!("{bytes} B")
    } else if bytes < MB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else if bytes < GB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    }
}

// --- Helpers ---------------------------------------------------------

/// UTC decomposition of a unix timestamp. Mirrors `gmtime`
/// from libc but is self-contained and pure. Uses the
/// "days-since-1970-01-01 → year/month/day" algorithm
/// from Howard Hinnant's `chrono-Compatible` paper.
fn unix_secs_to_ymdhms(unix_secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let secs = unix_secs;
    let days = (secs / 86_400) as i64;
    let secs_of_day = (secs % 86_400) as u32;
    let hour = secs_of_day / 3600;
    let min = (secs_of_day % 3600) / 60;
    let sec = secs_of_day % 60;

    // Hinnant's algorithm — civil_from_days. Returns
    // (year, month, day) for a day count where day 0 is
    // 1970-01-01.
    //
    // Constants:
    //   719_468  = days from 0000-03-01 to 1970-01-01
    //   146_097  = days in a 400-year Gregorian era
    //              (400*365 + 97 leap days)
    //   36_524   = days in a 100-year non-leap span
    //              (100*365 + 24 leap days, ignoring the
    //              400-year correction)
    //   146_096  = 146_097 - 1 (used as a bias in the
    //              negative-z branch to avoid C/C++ style
    //              truncated-modulo surprises; in Rust
    //              this is unused because integer division
    //              truncates toward zero, but we keep it
    //              for clarity)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_097 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146_096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d, hour, min, sec)
}

fn default_pictures_dir() -> PathBuf {
    if let Ok(p) = std::env::var("XDG_PICTURES_DIR")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join("Pictures");
    }
    // Last-ditch fallback: a relative directory. The
    // infrastructure layer's `ensure_dir_exists` will
    // surface a clean error if it can't be created.
    PathBuf::from("./Pictures")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- SnapshotConfig / Default -------------------------------

    #[test]
    fn default_quality_is_visually_lossless() {
        // 85 is the JPEG "visually lossless" sweet spot;
        // we don't want the default to be 100 (larger
        // files, no perceptual gain) or 70 (visible
        // artifacts on text / hard edges).
        assert_eq!(SnapshotConfig::default().quality, 85);
    }

    #[test]
    fn default_burst_is_single_shot() {
        // F12 (no modifier) is the common case; bursts
        // require an explicit Ctrl modifier.
        assert_eq!(SnapshotConfig::default().burst_count, 1);
    }

    #[test]
    fn default_dir_lives_under_pictures() {
        let dir = SnapshotConfig::default().dir;
        assert!(
            dir.ends_with("rust-rtsp-viewer"),
            "default dir should end in rust-rtsp-viewer, got {dir:?}"
        );
    }

    // ---- validate_config ----------------------------------------

    #[test]
    fn validate_accepts_a_sensible_default() {
        assert!(validate_config(&SnapshotConfig::default()).is_ok());
    }

    #[test]
    fn validate_rejects_quality_zero() {
        let cfg = SnapshotConfig {
            quality: 0,
            ..SnapshotConfig::default()
        };
        match validate_config(&cfg).unwrap_err() {
            SnapshotError::InvalidQuality(0) => {}
            e => panic!("expected InvalidQuality(0), got {e:?}"),
        }
    }

    #[test]
    fn validate_rejects_burst_count_zero() {
        let cfg = SnapshotConfig {
            burst_count: 0,
            ..SnapshotConfig::default()
        };
        match validate_config(&cfg).unwrap_err() {
            SnapshotError::InvalidBurstCount(0) => {}
            e => panic!("expected InvalidBurstCount(0), got {e:?}"),
        }
    }

    #[test]
    fn validate_rejects_burst_count_above_50() {
        let cfg = SnapshotConfig {
            burst_count: 51,
            ..SnapshotConfig::default()
        };
        match validate_config(&cfg).unwrap_err() {
            SnapshotError::InvalidBurstCount(51) => {}
            e => panic!("expected InvalidBurstCount(51), got {e:?}"),
        }
    }

    #[test]
    fn validate_rejects_empty_dir() {
        let cfg = SnapshotConfig {
            dir: PathBuf::new(),
            ..SnapshotConfig::default()
        };
        match validate_config(&cfg).unwrap_err() {
            SnapshotError::InvalidDir { .. } => {}
            e => panic!("expected InvalidDir, got {e:?}"),
        }
    }

    #[test]
    fn validate_accepts_burst_count_at_the_max() {
        let cfg = SnapshotConfig {
            burst_count: 50,
            ..SnapshotConfig::default()
        };
        assert!(validate_config(&cfg).is_ok());
    }

    // ---- generate_filename --------------------------------------

    #[test]
    fn filename_includes_app_prefix_and_extension() {
        let f = generate_filename(1_749_310_212, 1);
        assert!(f.starts_with("rust-rtsp-viewer-"), "got: {f}");
        assert!(f.ends_with(".png"), "got: {f}");
    }

    #[test]
    fn filename_decomposes_unix_timestamp_correctly() {
        // 2025-06-07 14:30:12 UTC = 1_749_306_612.
        // (Verified: 2025-01-01 00:00:00 UTC = 1_735_689_600;
        // add 31+28+31+30+31 = 151 days to reach 2025-06-01,
        // then 6 days, then 14:30:12.)
        let f = generate_filename(1_749_306_612, 1);
        assert_eq!(f, "rust-rtsp-viewer-2025-06-07-143012-001.png");
    }

    #[test]
    fn filename_sequence_is_zero_padded_to_three_digits() {
        // 1 → "001", 42 → "042", 100 → "100".
        assert_eq!(
            generate_filename(1_749_306_612, 1),
            "rust-rtsp-viewer-2025-06-07-143012-001.png"
        );
        assert_eq!(
            generate_filename(1_749_306_612, 42),
            "rust-rtsp-viewer-2025-06-07-143012-042.png"
        );
        assert_eq!(
            generate_filename(1_749_306_612, 100),
            "rust-rtsp-viewer-2025-06-07-143012-100.png"
        );
    }

    #[test]
    fn filename_handles_unix_epoch() {
        // 1970-01-01 00:00:00 UTC = 0.
        let f = generate_filename(0, 1);
        assert_eq!(f, "rust-rtsp-viewer-1970-01-01-000000-001.png");
    }

    #[test]
    fn filename_handles_leap_year_boundary() {
        // 2024 was a leap year. 2024-02-29 12:00:00 UTC =
        // 1_709_208_000.
        // (2024-01-01 = 1_704_067_200; +59 days to Feb 29
        // = +5_097_600; +12h = +43_200 → 1_709_208_000.)
        let f = generate_filename(1_709_208_000, 1);
        assert_eq!(f, "rust-rtsp-viewer-2024-02-29-120000-001.png");
    }

    // ---- compute_burst_timestamps -------------------------------

    #[test]
    fn burst_timestamps_start_at_zero() {
        let ts = compute_burst_timestamps(100, 5);
        assert_eq!(ts[0], 0, "first frame should be at t=0");
    }

    #[test]
    fn burst_timestamps_are_evenly_spaced() {
        let ts = compute_burst_timestamps(100, 5);
        assert_eq!(ts, vec![0, 100, 200, 300, 400]);
    }

    #[test]
    fn burst_timestamps_handle_count_one() {
        // Edge case: a burst of 1 is just a single frame
        // (no offset). Should still work, even though
        // the UI normally uses single-shot mode for
        // count=1.
        let ts = compute_burst_timestamps(100, 1);
        assert_eq!(ts, vec![0]);
    }

    #[test]
    fn burst_timestamps_handle_count_zero() {
        // Defensive: should never happen (validate_config
        // rejects count=0), but the function should still
        // return an empty Vec.
        assert!(compute_burst_timestamps(100, 0).is_empty());
    }

    // ---- render_notification ------------------------------------

    #[test]
    fn notification_includes_filename_and_size() {
        // 142_336 bytes = 139.0 KB (142_336 / 1024 = 139.0).
        let n = render_notification(
            "rust-rtsp-viewer-2025-06-07-143012-001.png",
            142_336,
            Path::new("/home/user/Pictures/rust-rtsp-viewer"),
        );
        assert!(n.contains("rust-rtsp-viewer-2025-06-07-143012-001.png"));
        assert!(n.contains("139.0 KB"), "got: {n}");
    }

    #[test]
    fn notification_includes_directory_on_a_separate_line() {
        let n = render_notification(
            "snap.png",
            1024,
            Path::new("/home/user/Pictures/rust-rtsp-viewer"),
        );
        // Two lines: headline + dir.
        let lines: Vec<&str> = n.lines().collect();
        assert_eq!(lines.len(), 2, "expected two lines, got {n:?}");
        assert!(
            lines[1].contains("/home/user/Pictures/rust-rtsp-viewer"),
            "second line should be the dir: {n}"
        );
    }

    #[test]
    fn notification_starts_with_a_check_glyph() {
        let n = render_notification("x.png", 100, Path::new("/tmp"));
        assert!(n.starts_with('✓'), "expected ✓ prefix, got: {n}");
    }

    // ---- humanize_bytes -----------------------------------------

    #[test]
    fn humanize_bytes_under_kb_uses_b() {
        assert_eq!(humanize_bytes(0), "0 B");
        assert_eq!(humanize_bytes(1023), "1023 B");
    }

    #[test]
    fn humanize_bytes_kb_range() {
        assert_eq!(humanize_bytes(1024), "1.0 KB");
        assert_eq!(humanize_bytes(1536), "1.5 KB");
    }

    #[test]
    fn humanize_bytes_mb_range() {
        assert_eq!(humanize_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(humanize_bytes(5 * 1024 * 1024 + 512 * 1024), "5.5 MB");
    }

    #[test]
    fn humanize_bytes_gb_range() {
        assert_eq!(humanize_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    // ---- ymdhms decomposition (via generate_filename) -----------

    #[test]
    fn ymdhms_handles_year_2000_rollover() {
        // 2000-01-01 00:00:00 UTC = 946_684_800.
        let f = generate_filename(946_684_800, 1);
        assert_eq!(f, "rust-rtsp-viewer-2000-01-01-000000-001.png");
    }

    #[test]
    fn ymdhms_handles_year_2100() {
        // 2100-01-01 00:00:00 UTC = 4_102_444_800.
        let f = generate_filename(4_102_444_800, 1);
        assert_eq!(f, "rust-rtsp-viewer-2100-01-01-000000-001.png");
    }
}
