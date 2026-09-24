use serde::Deserialize;

/// Available hardware encoder backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum HwEncoderBackend {
    /// Intel VA-API (e.g. vaapih264enc).
    VaApi,
    /// NVIDIA NVENC (e.g. nvh264enc).
    Nvenc,
    /// Raspberry Pi / V4L2 (e.g. v4l2h264enc).
    V4L2,
    /// Software fallback (e.g. x264enc).
    Software,
}

impl HwEncoderBackend {
    pub fn as_str(&self) -> &'static str {
        match self {
            HwEncoderBackend::VaApi => "vaapi",
            HwEncoderBackend::Nvenc => "nvenc",
            HwEncoderBackend::V4L2 => "v4l2",
            HwEncoderBackend::Software => "software",
        }
    }

    /// Returns the GStreamer encoder element name for this backend.
    pub fn encoder_element(&self) -> &'static str {
        match self {
            HwEncoderBackend::VaApi => "vaapih264enc",
            HwEncoderBackend::Nvenc => "nvh264enc",
            HwEncoderBackend::V4L2 => "v4l2h264enc",
            HwEncoderBackend::Software => "x264enc",
        }
    }
}

/// Hardware encoding configuration.
#[derive(Debug, Clone)]
pub struct HwEncoderConfig {
    /// Preferred backend. Default: auto-detect.
    pub preferred_backend: Option<HwEncoderBackend>,
    /// Bitrate in kbps. Default: 4000.
    pub bitrate_kbps: u32,
    /// Keyframe interval in seconds. Default: 2.
    pub keyframe_interval_secs: u32,
}

impl Default for HwEncoderConfig {
    fn default() -> Self {
        Self {
            preferred_backend: None,
            bitrate_kbps: 4000,
            keyframe_interval_secs: 2,
        }
    }
}

/// TOML mirror for `[hw_encoder]` section.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct HwEncoderConfigFile {
    pub preferred_backend: Option<String>,
    pub bitrate_kbps: Option<u32>,
    pub keyframe_interval_secs: Option<u32>,
}

impl HwEncoderConfigFile {
    pub fn into_config(self) -> HwEncoderConfig {
        let mut config = HwEncoderConfig::default();
        if let Some(b) = self.preferred_backend {
            config.preferred_backend = Some(match b.to_lowercase().as_str() {
                "vaapi" => HwEncoderBackend::VaApi,
                "nvenc" => HwEncoderBackend::Nvenc,
                "v4l2" => HwEncoderBackend::V4L2,
                "software" | "sw" => HwEncoderBackend::Software,
                _ => HwEncoderBackend::Software,
            });
        }
        if let Some(k) = self.bitrate_kbps { config.bitrate_kbps = k.clamp(100, 50000); }
        if let Some(k) = self.keyframe_interval_secs { config.keyframe_interval_secs = k.clamp(1, 30); }
        config
    }
}

/// Probe the system for available hardware encoders.
/// Returns the first available backend, or Software as fallback.
pub fn detect_backend(preferred: Option<HwEncoderBackend>) -> HwEncoderBackend {
    if let Some(p) = preferred {
        return p;
    }
    // Check in order of preference: VA-API, NVENC, V4L2, Software.
    // In a real implementation, we'd probe GStreamer registry.
    // For now, return Software as safe default.
    HwEncoderBackend::Software
}

/// Build the GStreamer encoder string for recording.
pub fn build_encoder_string(config: &HwEncoderConfig) -> String {
    let backend = detect_backend(config.preferred_backend);
    let element = backend.encoder_element();

    match backend {
        HwEncoderBackend::VaApi => {
            format!("{} rate-control=cbr bitrate={}", element, config.bitrate_kbps)
        }
        HwEncoderBackend::Nvenc => {
            format!("{} bitrate={} key-int-max={}",
                element, config.bitrate_kbps,
                config.keyframe_interval_secs)
        }
        HwEncoderBackend::V4L2 => {
            format!("{} extra-controls=\"encode,video_bitrate={}\"",
                element, config.bitrate_kbps * 1000)
        }
        HwEncoderBackend::Software => {
            format!("{} bitrate={} key-int-max={}",
                element, config.bitrate_kbps,
                config.keyframe_interval_secs)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_encoder_elements() {
        assert_eq!(HwEncoderBackend::VaApi.encoder_element(), "vaapih264enc");
        assert_eq!(HwEncoderBackend::Nvenc.encoder_element(), "nvh264enc");
        assert_eq!(HwEncoderBackend::V4L2.encoder_element(), "v4l2h264enc");
        assert_eq!(HwEncoderBackend::Software.encoder_element(), "x264enc");
    }

    #[test]
    fn backend_as_str() {
        assert_eq!(HwEncoderBackend::VaApi.as_str(), "vaapi");
        assert_eq!(HwEncoderBackend::Software.as_str(), "software");
    }

    #[test]
    fn default_config() {
        let c = HwEncoderConfig::default();
        assert!(c.preferred_backend.is_none());
        assert_eq!(c.bitrate_kbps, 4000);
        assert_eq!(c.keyframe_interval_secs, 2);
    }

    #[test]
    fn config_file_overrides() {
        let f = HwEncoderConfigFile {
            preferred_backend: Some("vaapi".into()),
            bitrate_kbps: Some(8000),
            keyframe_interval_secs: Some(4),
        };
        let c = f.into_config();
        assert_eq!(c.preferred_backend, Some(HwEncoderBackend::VaApi));
        assert_eq!(c.bitrate_kbps, 8000);
        assert_eq!(c.keyframe_interval_secs, 4);
    }

    #[test]
    fn config_file_clamps() {
        let f = HwEncoderConfigFile {
            bitrate_kbps: Some(10),
            keyframe_interval_secs: Some(100),
            ..Default::default()
        };
        let c = f.into_config();
        assert_eq!(c.bitrate_kbps, 100);
        assert_eq!(c.keyframe_interval_secs, 30);
    }

    #[test]
    fn detect_backend_uses_preferred() {
        assert_eq!(detect_backend(Some(HwEncoderBackend::Nvenc)), HwEncoderBackend::Nvenc);
    }

    #[test]
    fn detect_backend_fallback_software() {
        assert_eq!(detect_backend(None), HwEncoderBackend::Software);
    }

    #[test]
    fn build_encoder_string_software() {
        let config = HwEncoderConfig { preferred_backend: Some(HwEncoderBackend::Software), ..Default::default() };
        let s = build_encoder_string(&config);
        assert!(s.starts_with("x264enc"));
        assert!(s.contains("bitrate=4000"));
    }

    #[test]
    fn build_encoder_string_vaapi() {
        let config = HwEncoderConfig { preferred_backend: Some(HwEncoderBackend::VaApi), ..Default::default() };
        let s = build_encoder_string(&config);
        assert!(s.starts_with("vaapih264enc"));
        assert!(s.contains("bitrate=4000"));
    }
}
