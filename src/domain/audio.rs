//! Audio domain — pure types, state machine, and helpers
//! for the audio branch of the pipeline (item 11 of
//! the 14-item feature plan).
//!
//! The audio infrastructure (volume element control,
//! level message parsing) lives in
//! `crate::infrastructure::audio`; this module owns
//! the *semantics* of "is audio enabled?", "is it
//! muted?", "what's the volume?" independently of
//! any GStreamer knowledge.
//!
//! State machine:
//! ```text
//!        ┌─────── mute() ────────┐
//!        │                       ▼
//!   ┌────────┐               ┌────────┐
//!   │  Live  │ ── unmute() ─▶│ Muted  │
//!   └────────┘ ◀── mute() ── └────────┘
//!        ▲                       │
//!        └───────── toggle() ────┘
//! ```
//!
//! Note: `AudioState` is a *runtime* state
//! machine — distinct from the pipeline
//! configuration. The pipeline always has the
//! audio elements (item 11 design — uniform
//! pipeline, no rebuild on toggle); only the
//! `volume` element's property and the
//! `AudioState` flag change.

// Many of this module's public items (validate,
// mute, unmute, toggle, format) are exercised by
// the unit tests in this file and consumed by
// `crate::infrastructure::audio`. The dead_code
// lint would otherwise fire on items that are
// part of the public API but not yet used in the
// GTK wiring (item 11 final wiring, which is
// done in the same commit). Silencing at the
// module level is the cleanest way to keep the
// warnings down.
#![allow(dead_code)]

use thiserror::Error;

/// User-facing configuration for the audio branch.
/// Resolved from CLI / `config.toml` / defaults in
/// `main.rs`.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioConfig {
    /// Whether the audio branch is enabled. When
    /// `false`, the volume element is set to 0
    /// (effectively muted) and no audio is
    /// rendered. The pipeline still has the audio
    /// elements — only the volume changes.
    pub enabled: bool,
    /// Initial volume, `0.0..=1.0`. 0.0 = silent,
    /// 1.0 = full. The `volume` GStreamer element
    /// accepts values in the same range, with 1.0
    /// being the "no attenuation" reference.
    pub volume: f32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        // Audio is enabled by default — the
        // majority of cameras stream audio, and
        // the user opted into playing a stream
        // (which usually includes its audio
        // track). Users who want silence can
        // press `m` or set `enabled = false` in
        // the config.
        Self {
            enabled: true,
            volume: 0.8,
        }
    }
}

impl AudioConfig {
    /// Validate the config. Returns `Ok(())` if
    /// the config is usable, `AudioError::Config`
    /// with a human-readable message otherwise.
    /// Cheap to call (no I/O).
    pub fn validate(&self) -> Result<(), AudioError> {
        if !self.volume.is_finite() {
            return Err(AudioError::Config(format!(
                "volume must be finite (got {})",
                self.volume
            )));
        }
        if self.volume < 0.0 || self.volume > 1.0 {
            return Err(AudioError::Config(format!(
                "volume must be in 0.0..=1.0 (got {})",
                self.volume
            )));
        }
        Ok(())
    }
}

/// Errors that can be produced by the audio
/// domain. `Config` is for `validate_config`
/// rejections; `InvalidState` covers state-machine
/// violations (shouldn't happen if the API is used
/// correctly, but kept for symmetry with the
/// recording module).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AudioError {
    #[error("audio config error: {0}")]
    Config(String),
    #[error("invalid audio state transition: {0}")]
    InvalidState(&'static str),
}

/// Runtime state of the audio branch. The
/// pipeline always has the audio elements; this
/// enum is a UI-side tracker that the sidebar
/// reads and the volume property mirrors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioState {
    /// Audio is enabled and the volume is
    /// non-zero. `volume_x1000` is the current
    /// volume in `0..=1000` (i.e. 0.0..=1.0
    /// multiplied by 1000, truncated to integer
    /// for display). The integer form keeps
    /// the struct `Eq`-derivable; the actual
    /// `f32` volume is only meaningful when
    /// handed to the `volume` GStreamer element.
    Live { volume_x1000: u32 },
    /// Audio is muted (volume = 0). Mirrors the
    /// state where the user pressed `m` (or set
    /// `enabled = false` in the config).
    Muted,
}

impl AudioState {
    /// True if the state is `Muted`. Convenience
    /// for the sidebar.
    pub fn is_muted(&self) -> bool {
        matches!(self, Self::Muted)
    }

    /// Transition to `Muted`. No-op if already
    /// muted.
    pub fn mute(&self) -> Self {
        match self {
            Self::Muted => Self::Muted,
            Self::Live { .. } => Self::Muted,
        }
    }

    /// Transition to `Live` with the given
    /// volume. The volume is clamped to
    /// `0..=1000` (i.e. `0.0..=1.0` × 1000).
    pub fn unmute(&self, volume_x1000: u32) -> Self {
        Self::Live {
            volume_x1000: volume_x1000.min(1000),
        }
    }

    /// Toggle. Returns the new state plus the
    /// volume that should be set on the
    /// GStreamer `volume` element (caller passes
    /// this to `volume_element.set_property("volume", ...)`).
    /// When transitioning to `Muted`, the
    /// returned volume is 0; when transitioning
    /// to `Live`, the returned volume is the
    /// state's stored volume (or 800 = 0.8 if
    /// the state was previously Muted without a
    /// stored volume).
    pub fn toggle(&self) -> (Self, u32) {
        match self {
            Self::Muted => (Self::Live { volume_x1000: 800 }, 800),
            Self::Live { volume_x1000: _ } => (Self::Muted, 0),
        }
    }

    /// The current volume in `0..=1000` (0 =
    /// muted, 1000 = full). Always returns 0
    /// for `Muted` and `volume_x1000` for
    /// `Live`.
    pub fn volume_x1000(&self) -> u32 {
        match self {
            Self::Muted => 0,
            Self::Live { volume_x1000 } => *volume_x1000,
        }
    }

    /// The current volume in `0.0..=1.0`
    /// (convenience for the GStreamer
    /// `volume` element's `volume` property,
    /// which is an `f32` in `[0, 10]` but we
    /// restrict to `[0, 1]`). Returns 0.0 for
    /// `Muted`, `volume_x1000 / 1000.0` for
    /// `Live`.
    pub fn volume_f32(&self) -> f32 {
        self.volume_x1000() as f32 / 1000.0
    }

    /// True if the state is `Live` with a
    /// non-zero volume. The opposite of
    /// `is_muted` — used by the notification
    /// builder to choose between the
    /// "▶ unmuted" and "■ muted" headlines.
    pub fn is_audible(&self) -> bool {
        match self {
            Self::Muted => false,
            Self::Live { volume_x1000 } => *volume_x1000 > 0,
        }
    }
}

impl Default for AudioState {
    fn default() -> Self {
        // Default state: Live at 0.8. This is
        // mirrored in `AudioConfig::default()`;
        // the two defaults are kept consistent.
        Self::Live { volume_x1000: 800 }
    }
}

/// Format a `0..=1000` volume as a `0.00..=1.00`
/// string for the sidebar (e.g. `0.80`). 2 decimal
/// places — enough resolution for the user to
/// feel changes but not so much that the sidebar
/// flickers with sub-1% changes.
pub fn format_volume(volume_x1000: u32) -> String {
    format!("{:.2}", volume_x1000 as f32 / 1000.0)
}

/// Convert an `f32` volume (0.0..=1.0) to the
/// `0..=1000` integer form used by `AudioState`.
/// Saturates to 0 / 1000 if out of range. Returns
/// 0 for NaN, `-INFINITY`, and 0.0; 1000 for
/// `+INFINITY` and > 1.0.
pub fn volume_to_x1000(volume: f32) -> u32 {
    if volume.is_nan() {
        return 0;
    }
    if volume >= 1.0 {
        // Also catches +INFINITY.
        return 1000;
    }
    if volume <= 0.0 {
        // Also catches -INFINITY.
        return 0;
    }
    (volume * 1000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- AudioConfig ---

    #[test]
    fn default_config_has_sensible_values() {
        let cfg = AudioConfig::default();
        assert!(cfg.enabled);
        assert!((cfg.volume - 0.8).abs() < 1e-6);
    }

    #[test]
    fn validate_accepts_default_config() {
        assert!(AudioConfig::default().validate().is_ok());
    }

    #[test]
    fn validate_accepts_boundary_values() {
        let mut cfg = AudioConfig {
            volume: 0.0,
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
        cfg.volume = 1.0;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_rejects_negative_volume() {
        let cfg = AudioConfig {
            volume: -0.1,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, AudioError::Config(_)));
        assert!(err.to_string().contains("volume"));
    }

    #[test]
    fn validate_rejects_above_one_volume() {
        let cfg = AudioConfig {
            volume: 1.5,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, AudioError::Config(_)));
    }

    #[test]
    fn validate_rejects_nan_volume() {
        let cfg = AudioConfig {
            volume: f32::NAN,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, AudioError::Config(_)));
    }

    #[test]
    fn validate_rejects_infinite_volume() {
        let cfg = AudioConfig {
            volume: f32::INFINITY,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(matches!(err, AudioError::Config(_)));
    }

    // --- AudioState ---

    #[test]
    fn default_state_is_live() {
        let s = AudioState::default();
        assert!(!s.is_muted());
        assert_eq!(s.volume_x1000(), 800);
    }

    #[test]
    fn mute_from_live_transitions_to_muted() {
        let s = AudioState::Live { volume_x1000: 500 };
        assert_eq!(s.mute(), AudioState::Muted);
    }

    #[test]
    fn mute_from_muted_is_noop() {
        let s = AudioState::Muted;
        assert_eq!(s.mute(), AudioState::Muted);
    }

    #[test]
    fn unmute_returns_live_with_volume() {
        let s = AudioState::Muted;
        assert_eq!(s.unmute(600), AudioState::Live { volume_x1000: 600 });
    }

    #[test]
    fn unmute_clamps_volume_to_max() {
        let s = AudioState::Muted;
        assert_eq!(s.unmute(1500), AudioState::Live { volume_x1000: 1000 });
    }

    #[test]
    fn toggle_from_luted_mutes_with_volume_zero() {
        let s = AudioState::Live { volume_x1000: 750 };
        let (next, vol) = s.toggle();
        assert_eq!(next, AudioState::Muted);
        assert_eq!(vol, 0);
    }

    #[test]
    fn toggle_from_muted_unmutes_to_080() {
        let s = AudioState::Muted;
        let (next, vol) = s.toggle();
        assert!(matches!(next, AudioState::Live { .. }));
        assert_eq!(vol, 800);
    }

    // --- format_volume / volume_to_x1000 ---

    #[test]
    fn format_volume_renders_two_decimals() {
        assert_eq!(format_volume(0), "0.00");
        assert_eq!(format_volume(500), "0.50");
        assert_eq!(format_volume(800), "0.80");
        assert_eq!(format_volume(1000), "1.00");
    }

    #[test]
    fn volume_to_x1000_basic() {
        assert_eq!(volume_to_x1000(0.0), 0);
        assert_eq!(volume_to_x1000(0.5), 500);
        assert_eq!(volume_to_x1000(0.8), 800);
        assert_eq!(volume_to_x1000(1.0), 1000);
    }

    #[test]
    fn volume_to_x1000_clamps_out_of_range() {
        assert_eq!(volume_to_x1000(-0.1), 0);
        assert_eq!(volume_to_x1000(1.5), 1000);
    }

    #[test]
    fn volume_to_x1000_handles_non_finite() {
        assert_eq!(volume_to_x1000(f32::NAN), 0);
        assert_eq!(volume_to_x1000(f32::INFINITY), 1000);
        assert_eq!(volume_to_x1000(f32::NEG_INFINITY), 0);
    }
}
