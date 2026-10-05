use serde::Deserialize;

use crate::domain::zones::ZoneConfig;

/// Whether cameras that are not on screen must keep decoding so motion can
/// still be seen.
///
/// Motion is computed from decoded frames, so a camera paused by
/// `[view] pause_hidden` is blind: it cannot start a motion recording or raise
/// a notification. That only matters when something *reacts* to motion
/// (`[recording] on_motion` or `[notifications]`); the on-screen indicator
/// alone does not justify decoding every camera. Detection must also be on.
pub fn needs_background_watch(
    motion_enabled: bool,
    on_motion_recording: bool,
    notifications_enabled: bool,
) -> bool {
    motion_enabled && (on_motion_recording || notifications_enabled)
}

/// Configuration for motion detection via frame differencing.
#[derive(Debug, Clone)]
pub struct MotionConfig {
    /// Whether motion detection is enabled.
    pub enabled: bool,
    /// Per-pixel absolute luma difference threshold (1–255).
    /// A pixel is considered "changed" if the luma difference exceeds this.
    pub threshold: u8,
    /// Minimum fraction of the frame (0.0–1.0) that must contain changed
    /// pixels for the frame to be classified as "motion detected".
    pub contour_area: f64,
    /// Subsampling stride for the frame comparison. Higher = faster but
    /// coarser. 1 = full resolution, 8 = sample every 8th pixel.
    pub sample_stride: usize,
}

impl Default for MotionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: 25,
            contour_area: 0.005,
            sample_stride: 8,
        }
    }
}

/// TOML mirror for `[motion]` section. All fields optional.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct MotionConfigFile {
    pub enabled: Option<bool>,
    pub threshold: Option<u8>,
    pub contour_area: Option<f64>,
    pub sample_stride: Option<usize>,
}

impl MotionConfigFile {
    pub fn into_config(self) -> MotionConfig {
        let mut config = MotionConfig::default();
        if let Some(e) = self.enabled {
            config.enabled = e;
        }
        if let Some(t) = self.threshold {
            config.threshold = t.clamp(1, 255);
        }
        if let Some(c) = self.contour_area {
            config.contour_area = c.clamp(0.0, 1.0);
        }
        if let Some(s) = self.sample_stride {
            config.sample_stride = s.clamp(1, 32);
        }
        config
    }
}

/// Result of a motion detection comparison between two frames.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionResult {
    /// Fraction of sampled pixels that changed (0.0–1.0).
    pub motion_level: f64,
    /// Whether the motion level exceeds the contour area threshold.
    pub motion_active: bool,
    /// Number of changed pixels counted (before normalization).
    pub changed_pixels: u64,
    /// Total number of sampled pixels.
    pub total_sampled: u64,
}

/// Compute luma for an RGBA pixel using BT.601 weights.
#[inline]
fn rgba_luma(r: u8, g: u8, b: u8) -> u32 {
    // Use integer math: 0.299*256 ≈ 77, 0.587*256 ≈ 150, 0.114*256 ≈ 29
    // luma = (77*R + 150*G + 29*B) / 256
    (77u32 * r as u32 + 150u32 * g as u32 + 29u32 * b as u32) / 256
}

/// Detect motion between two RGBA frames using subsampled frame differencing.
///
/// Both `prev` and `curr` must have the same dimensions (`width × height`),
/// with each pixel stored as 4 consecutive bytes (R, G, B, A).
///
/// The algorithm:
/// 1. Subsample both frames at the configured stride.
/// 2. Compute luma (BT.601) for each sampled pixel in both frames.
/// 3. Count pixels where `|luma_curr - luma_prev| > threshold`.
/// 4. `motion_level` = changed_pixels / total_sampled.
/// 5. `motion_active` = motion_level > contour_area.
///
/// If `zone_config` is provided and has active zones, only pixels inside
/// those zones are sampled: `motion_level` is the changed fraction of the zone.
pub fn detect_motion(
    prev: &[u8],
    curr: &[u8],
    width: usize,
    height: usize,
    config: &MotionConfig,
    zone_config: Option<&ZoneConfig>,
) -> Option<MotionResult> {
    if prev.len() != curr.len() || width == 0 || height == 0 {
        return None;
    }

    let stride = config.sample_stride;
    let threshold = config.threshold as i64;
    let mut changed: u64 = 0;
    let mut total: u64 = 0;
    let active_zones = zone_config.filter(|z| z.has_active());

    let mut row = 0;
    while row < height {
        let row_start = row * width * 4;
        let mut col = 0;
        while col < width {
            let idx = row_start + col * 4;
            if idx + 2 < prev.len() && idx + 2 < curr.len() {
                let prev_luma = rgba_luma(prev[idx], prev[idx + 1], prev[idx + 2]) as i64;
                let curr_luma = rgba_luma(curr[idx], curr[idx + 1], curr[idx + 2]) as i64;
                let diff = (curr_luma - prev_luma).unsigned_abs() as i64;

                // With zones, only pixels inside them are sampled at all, so
                // the level is "fraction of the zone that changed". Counting
                // outside pixels in `total` would make a small zone unable to
                // ever reach `contour_area`.
                let in_scope = active_zones.is_none_or(|zones| {
                    zones.is_motion_allowed(crate::domain::zones::Point::new(
                        col as f64 / width as f64,
                        row as f64 / height as f64,
                    ))
                });
                if in_scope {
                    total += 1;
                    if diff > threshold {
                        changed += 1;
                    }
                }
            }
            col += stride;
        }
        row += stride;
    }

    if total == 0 {
        return None;
    }

    let motion_level = changed as f64 / total as f64;
    let motion_active = motion_level > config.contour_area;

    Some(MotionResult {
        motion_level,
        motion_active,
        changed_pixels: changed,
        total_sampled: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_watch_needs_detection_and_a_reaction() {
        // detection off: nothing to watch for
        assert!(!needs_background_watch(false, true, true));
        // detection on, nothing reacts: the indicator alone is not enough
        assert!(!needs_background_watch(true, false, false));
        // detection on and something reacts to it
        assert!(needs_background_watch(true, true, false));
        assert!(needs_background_watch(true, false, true));
        assert!(needs_background_watch(true, true, true));
    }
    use crate::domain::zones::{MotionZone, Point, ZoneConfig};

    fn make_frame(width: usize, height: usize, fill: u8) -> Vec<u8> {
        vec![fill; width * height * 4]
    }

    #[test]
    fn detect_motion_returns_none_on_size_mismatch() {
        let a = make_frame(10, 10, 128);
        let b = make_frame(20, 10, 128);
        assert!(detect_motion(&a, &b, 10, 10, &MotionConfig::default(), None).is_none());
    }

    #[test]
    fn detect_motion_returns_none_on_empty_dimensions() {
        let a = make_frame(0, 0, 128);
        let b = make_frame(0, 0, 128);
        assert!(detect_motion(&a, &b, 0, 0, &MotionConfig::default(), None).is_none());
    }

    #[test]
    fn no_motion_on_identical_frames() {
        let frame = make_frame(64, 64, 128);
        let result = detect_motion(&frame, &frame, 64, 64, &MotionConfig::default(), None).unwrap();
        assert_eq!(result.changed_pixels, 0);
        assert!(!result.motion_active);
    }

    #[test]
    fn motion_on_completely_different_frames() {
        let dark = make_frame(64, 64, 0);
        let bright = make_frame(64, 64, 255);
        let result = detect_motion(&dark, &bright, 64, 64, &MotionConfig::default(), None).unwrap();
        assert!(result.motion_level > 0.9, "got {}", result.motion_level);
        assert!(result.motion_active);
    }

    #[test]
    fn motion_level_is_fraction() {
        let a = make_frame(64, 64, 100);
        let mut b = make_frame(64, 64, 100);
        for y in 0..32 {
            for x in 0..32 {
                let idx = (y * 64 + x) * 4;
                b[idx] = 255;
            }
        }
        let config = MotionConfig {
            sample_stride: 1,
            ..MotionConfig::default()
        };
        let result = detect_motion(&a, &b, 64, 64, &config, None).unwrap();
        assert!(result.motion_level > 0.15, "got {}", result.motion_level);
        assert!(result.motion_active);
    }

    #[test]
    fn small_change_below_threshold() {
        let a = make_frame(64, 64, 128);
        let b = make_frame(64, 64, 128);
        let result = detect_motion(&a, &b, 64, 64, &MotionConfig::default(), None).unwrap();
        assert!(!result.motion_active);
    }

    #[test]
    fn contour_area_threshold() {
        let a = make_frame(64, 64, 0);
        let mut b = make_frame(64, 64, 0);
        for y in 0..2 {
            for x in 0..2 {
                let idx = (y * 64 + x) * 4;
                b[idx] = 255;
            }
        }
        let config = MotionConfig {
            sample_stride: 1,
            contour_area: 0.5,
            ..MotionConfig::default()
        };
        let result = detect_motion(&a, &b, 64, 64, &config, None).unwrap();
        assert!(!result.motion_active);
    }

    #[test]
    fn luma_weighted_differently_for_rgb() {
        let mut red = make_frame(64, 64, 0);
        for i in (0..red.len()).step_by(4) {
            red[i] = 255;
        }
        let mut green = make_frame(64, 64, 0);
        for i in (0..green.len()).step_by(4) {
            green[i + 1] = 255;
        }
        let config = MotionConfig {
            threshold: 20,
            sample_stride: 1,
            ..MotionConfig::default()
        };
        let result = detect_motion(&red, &green, 64, 64, &config, None).unwrap();
        assert!(result.motion_active, "red→green should be detected");
    }

    #[test]
    fn custom_config_applies() {
        let a = make_frame(64, 64, 100);
        let b = make_frame(64, 64, 110);
        let config = MotionConfig {
            threshold: 50,
            sample_stride: 1,
            ..MotionConfig::default()
        };
        let result = detect_motion(&a, &b, 64, 64, &config, None).unwrap();
        assert_eq!(result.changed_pixels, 0);
        assert!(!result.motion_active);
    }

    #[test]
    fn zone_filtering_reduces_motion() {
        let a = make_frame(64, 64, 0);
        let mut b = make_frame(64, 64, 0);
        // Change only bottom-right quadrant
        for y in 32..64 {
            for x in 32..64 {
                let idx = (y * 64 + x) * 4;
                b[idx] = 255;
            }
        }
        let config = MotionConfig {
            sample_stride: 1,
            ..MotionConfig::default()
        };
        // Zone covering only top-left quadrant (0..0.5, 0..0.5)
        let zone_config = ZoneConfig {
            zones: vec![MotionZone::new(
                "top-left",
                vec![
                    Point::new(0.0, 0.0),
                    Point::new(0.5, 0.0),
                    Point::new(0.5, 0.5),
                    Point::new(0.0, 0.5),
                ],
            )],
        };
        // Without zones: ~25% motion detected
        let without = detect_motion(&a, &b, 64, 64, &config, None).unwrap();
        assert!(
            without.motion_level > 0.15,
            "without zones: {}",
            without.motion_level
        );
        // With zone filtering: 0% motion (change is outside the zone)
        let with_zone = detect_motion(&a, &b, 64, 64, &config, Some(&zone_config)).unwrap();
        assert_eq!(
            with_zone.changed_pixels, 0,
            "zone should filter out all changes"
        );
        assert!(!with_zone.motion_active);
    }

    #[test]
    fn zone_filtering_passes_motion_inside_zone() {
        let a = make_frame(64, 64, 0);
        let mut b = make_frame(64, 64, 0);
        // Change only top-left quadrant
        for y in 0..32 {
            for x in 0..32 {
                let idx = (y * 64 + x) * 4;
                b[idx] = 255;
            }
        }
        let config = MotionConfig {
            sample_stride: 1,
            ..MotionConfig::default()
        };
        let zone_config = ZoneConfig {
            zones: vec![MotionZone::new(
                "top-left",
                vec![
                    Point::new(0.0, 0.0),
                    Point::new(0.5, 0.0),
                    Point::new(0.5, 0.5),
                    Point::new(0.0, 0.5),
                ],
            )],
        };
        let result = detect_motion(&a, &b, 64, 64, &config, Some(&zone_config)).unwrap();
        assert!(
            result.motion_level > 0.15,
            "zone should pass motion: {}",
            result.motion_level
        );
    }

    #[test]
    fn small_zone_fully_changed_triggers_motion() {
        // Zone covers ~1% of the frame; if it changes entirely the level must
        // be measured against the zone, not the whole frame.
        let a = make_frame(100, 100, 0);
        let mut b = make_frame(100, 100, 0);
        for y in 0..10 {
            for x in 0..10 {
                b[(y * 100 + x) * 4] = 255;
            }
        }
        let config = MotionConfig {
            sample_stride: 1,
            ..MotionConfig::default()
        };
        let zone_config = ZoneConfig {
            zones: vec![MotionZone::new(
                "corner",
                vec![
                    Point::new(0.0, 0.0),
                    Point::new(0.1, 0.0),
                    Point::new(0.1, 0.1),
                    Point::new(0.0, 0.1),
                ],
            )],
        };
        let r = detect_motion(&a, &b, 100, 100, &config, Some(&zone_config)).unwrap();
        assert!(r.motion_active, "level {}", r.motion_level);
        assert!(r.total_sampled < 10_000);
    }

    #[test]
    fn config_file_into_config_defaults() {
        let f = MotionConfigFile::default();
        let c = f.into_config();
        assert!(c.enabled);
        assert_eq!(c.threshold, 25);
        assert!((c.contour_area - 0.005).abs() < 1e-9);
        assert_eq!(c.sample_stride, 8);
    }

    #[test]
    fn config_file_into_config_overrides() {
        let f = MotionConfigFile {
            enabled: Some(false),
            threshold: Some(10),
            contour_area: Some(0.1),
            sample_stride: Some(4),
        };
        let c = f.into_config();
        assert!(!c.enabled);
        assert_eq!(c.threshold, 10);
        assert!((c.contour_area - 0.1).abs() < 1e-9);
        assert_eq!(c.sample_stride, 4);
    }

    #[test]
    fn config_file_clamps_values() {
        let f = MotionConfigFile {
            threshold: Some(0),
            contour_area: Some(2.0),
            sample_stride: Some(0),
            ..MotionConfigFile::default()
        };
        let c = f.into_config();
        assert_eq!(c.threshold, 1);
        assert!((c.contour_area - 1.0).abs() < 1e-9);
        assert_eq!(c.sample_stride, 1);
    }
}
