//! Per-camera runtime status, shared by the video engine and the UI.
//!
//! Pure data: no `iced`, no GStreamer. The engine reports a [`StatusReading`];
//! the UI decides how to show it.

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

use crate::i18n::t;

impl CameraStatus {
    pub fn label(&self) -> &'static str {
        match self {
            CameraStatus::Live => "LIVE",
            CameraStatus::Offline => "OFFLINE",
            CameraStatus::Reconnecting => "RECONNECTING",
            CameraStatus::Recording => "RECORDING",
            CameraStatus::Disabled => t("DESATIVADA"),
            CameraStatus::Connecting => "CONNECTING",
            CameraStatus::Paused => "PAUSED",
        }
    }

    /// Sentence-case Portuguese label for the UI (the all-caps `label` reads as
    /// shouting).
    pub fn label_pt(&self) -> &'static str {
        match self {
            CameraStatus::Live => t("Ao vivo"),
            CameraStatus::Offline => t("Offline"),
            CameraStatus::Reconnecting => t("Reconectando"),
            CameraStatus::Recording => t("Gravando"),
            CameraStatus::Disabled => t("Desativada"),
            CameraStatus::Connecting => t("Conectando\u{2026}"),
            CameraStatus::Paused => t("Pausada"),
        }
    }
}

/// What the engine currently reports for a camera's bitrate column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitrateReading {
    /// The pipeline reported an error; show its text instead of a rate.
    Error(String),
    /// Compressed rate in kbit/s (`0` = no data yet).
    Kbps(u64),
}

/// A sample of a camera's health taken from its pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusReading {
    pub fps: f64,
    /// The new status, or `None` to leave the previous one in place.
    pub status: Option<CameraStatus>,
    /// The bitrate to show, or `None` to leave the previous one in place.
    pub bitrate: Option<BitrateReading>,
}

impl BitrateReading {
    /// Short text for a table cell: `—`, `640k`, `1.2M`, or the error itself.
    pub fn display(&self) -> String {
        match self {
            BitrateReading::Error(e) => e.clone(),
            BitrateReading::Kbps(0) => "\u{2014}".into(),
            BitrateReading::Kbps(k) if *k >= 1000 => format!("{:.1}M", *k as f64 / 1000.0),
            BitrateReading::Kbps(k) => format!("{k}k"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_text_matches_the_old_formatting() {
        assert_eq!(BitrateReading::Kbps(0).display(), "\u{2014}");
        assert_eq!(BitrateReading::Kbps(640).display(), "640k");
        assert_eq!(BitrateReading::Kbps(1000).display(), "1.0M");
        assert_eq!(BitrateReading::Kbps(2560).display(), "2.6M");
        assert_eq!(BitrateReading::Error("boom".into()).display(), "boom");
    }

    #[test]
    fn labels_are_stable() {
        assert_eq!(CameraStatus::Live.label(), "LIVE");
        assert_eq!(CameraStatus::Paused.label_pt(), "Pausada");
    }
}
