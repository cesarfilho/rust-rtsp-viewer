use serde::Deserialize;

/// Configuration for bidirectional audio (microphone input).
#[derive(Debug, Clone)]
pub struct BidirectionalAudioConfig {
    /// Whether bidirectional audio is enabled.
    pub enabled: bool,
    /// Microphone device name (None = default device).
    pub device: Option<String>,
    /// Input volume 0.0..=1.0.
    pub volume: f32,
    /// Audio encoding: "opus" (default) or "pcm".
    pub encoding: AudioEncoding,
}

impl Default for BidirectionalAudioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            device: None,
            volume: 0.8,
            encoding: AudioEncoding::Opus,
        }
    }
}

/// Audio encoding for microphone input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AudioEncoding {
    Opus,
    Pcm,
}

impl AudioEncoding {
    pub fn as_str(&self) -> &'static str {
        match self {
            AudioEncoding::Opus => "opus",
            AudioEncoding::Pcm => "pcm",
        }
    }
}

/// TOML mirror for `[bidirectional_audio]` section.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct BidirectionalAudioConfigFile {
    pub enabled: Option<bool>,
    pub device: Option<String>,
    pub volume: Option<f32>,
    pub encoding: Option<String>,
}

impl BidirectionalAudioConfigFile {
    pub fn into_config(self) -> BidirectionalAudioConfig {
        let mut config = BidirectionalAudioConfig::default();
        if let Some(e) = self.enabled {
            config.enabled = e;
        }
        if let Some(d) = self.device {
            config.device = Some(d);
        }
        if let Some(v) = self.volume {
            config.volume = v.clamp(0.0, 1.0);
        }
        if let Some(enc) = self.encoding {
            config.encoding = match enc.to_lowercase().as_str() {
                "pcm" => AudioEncoding::Pcm,
                _ => AudioEncoding::Opus,
            };
        }
        config
    }
}

/// Runtime state for bidirectional audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MicState {
    /// Microphone is muted.
    #[default]
    Muted,
    /// Microphone is active (transmitting).
    Active,
}

impl MicState {
    pub fn toggle(self) -> Self {
        match self {
            MicState::Muted => MicState::Active,
            MicState::Active => MicState::Muted,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            MicState::Muted => "Muted",
            MicState::Active => "Active",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_disabled() {
        let c = BidirectionalAudioConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.volume, 0.8);
        assert_eq!(c.encoding, AudioEncoding::Opus);
    }

    #[test]
    fn mic_toggle() {
        assert_eq!(MicState::Muted.toggle(), MicState::Active);
        assert_eq!(MicState::Active.toggle(), MicState::Muted);
    }

    #[test]
    fn mic_label() {
        assert_eq!(MicState::Muted.label(), "Muted");
        assert_eq!(MicState::Active.label(), "Active");
    }

    #[test]
    fn encoding_as_str() {
        assert_eq!(AudioEncoding::Opus.as_str(), "opus");
        assert_eq!(AudioEncoding::Pcm.as_str(), "pcm");
    }

    #[test]
    fn config_file_overrides() {
        let f = BidirectionalAudioConfigFile {
            enabled: Some(true),
            device: Some("hw:1".into()),
            volume: Some(0.5),
            encoding: Some("pcm".into()),
        };
        let c = f.into_config();
        assert!(c.enabled);
        assert_eq!(c.device.as_deref(), Some("hw:1"));
        assert_eq!(c.volume, 0.5);
        assert_eq!(c.encoding, AudioEncoding::Pcm);
    }

    #[test]
    fn config_file_volume_clamped() {
        let f = BidirectionalAudioConfigFile {
            volume: Some(2.0),
            ..Default::default()
        };
        let c = f.into_config();
        assert_eq!(c.volume, 1.0);
    }
}
