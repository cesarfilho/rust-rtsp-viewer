use serde::Deserialize;

/// Which stream is currently active for a camera.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[derive(Default)]
pub enum StreamQuality {
    /// Full resolution stream (main).
    #[default]
    Main,
    /// Reduced resolution stream (sub/secondary).
    Sub,
}


impl StreamQuality {
    pub fn label(&self) -> &'static str {
        match self {
            StreamQuality::Main => "Main",
            StreamQuality::Sub => "Sub",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            StreamQuality::Main => StreamQuality::Sub,
            StreamQuality::Sub => StreamQuality::Main,
        }
    }
}

/// Configuration for multi-stream support on a camera.
#[derive(Debug, Clone, Deserialize)]
pub struct MultiStreamConfig {
    /// URL for the sub/secondary stream (e.g. lower resolution).
    pub sub_stream_url: Option<String>,
    /// Preferred starting quality. Default: Main.
    pub default_quality: StreamQuality,
}

impl Default for MultiStreamConfig {
    fn default() -> Self {
        Self {
            sub_stream_url: None,
            default_quality: StreamQuality::Main,
        }
    }
}

/// TOML mirror for `[multi_stream]` section.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct MultiStreamConfigFile {
    pub sub_stream_url: Option<String>,
    pub default_quality: Option<String>,
}

impl MultiStreamConfigFile {
    pub fn into_config(self) -> MultiStreamConfig {
        let mut config = MultiStreamConfig::default();
        if let Some(u) = self.sub_stream_url { config.sub_stream_url = Some(u); }
        if let Some(q) = self.default_quality {
            config.default_quality = match q.to_lowercase().as_str() {
                "sub" => StreamQuality::Sub,
                _ => StreamQuality::Main,
            };
        }
        config
    }
}

/// Given the current stream quality and multi-stream config,
/// return the URL to use.
pub fn stream_url_for_quality(
    main_url: &str,
    quality: StreamQuality,
    config: &MultiStreamConfig,
) -> String {
    match quality {
        StreamQuality::Main => main_url.to_string(),
        StreamQuality::Sub => config.sub_stream_url.clone().unwrap_or_else(|| main_url.to_string()),
    }
}

/// Which stream a camera should be decoding right now.
///
/// The sub-stream is for tiles: a 16-camera grid should not decode sixteen
/// 1080p mains. Anything that needs the full picture gets the main stream.
///
/// - `has_sub`: the camera has a `sub_url` configured; without one it is always Main.
/// - `recording`: a recording is running. Switching rebuilds the pipeline and
///   would cut the file, so a recording camera keeps whatever it is on.
/// - `large_view`: the camera fills the view (spotlight, flex main, or the
///   only camera on the page).
pub fn desired_quality(
    has_sub: bool,
    recording: bool,
    current: StreamQuality,
    large_view: bool,
) -> StreamQuality {
    if !has_sub {
        return StreamQuality::Main;
    }
    if recording {
        return current;
    }
    if large_view {
        StreamQuality::Main
    } else {
        StreamQuality::Sub
    }
}

/// Whether sub-stream switching is available for this camera.
pub fn has_sub_stream(config: &MultiStreamConfig) -> bool {
    config.sub_stream_url.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_sub_url_is_always_main() {
        for rec in [false, true] {
            for big in [false, true] {
                assert_eq!(
                    desired_quality(false, rec, StreamQuality::Sub, big),
                    StreamQuality::Main
                );
            }
        }
    }

    #[test]
    fn tiles_use_sub_and_large_views_use_main() {
        assert_eq!(desired_quality(true, false, StreamQuality::Main, false), StreamQuality::Sub);
        assert_eq!(desired_quality(true, false, StreamQuality::Sub, true), StreamQuality::Main);
    }

    #[test]
    fn a_recording_camera_never_switches() {
        assert_eq!(desired_quality(true, true, StreamQuality::Main, false), StreamQuality::Main);
        assert_eq!(desired_quality(true, true, StreamQuality::Sub, true), StreamQuality::Sub);
    }

    #[test]
    fn stream_quality_toggle() {
        assert_eq!(StreamQuality::Main.toggle(), StreamQuality::Sub);
        assert_eq!(StreamQuality::Sub.toggle(), StreamQuality::Main);
    }

    #[test]
    fn stream_quality_label() {
        assert_eq!(StreamQuality::Main.label(), "Main");
        assert_eq!(StreamQuality::Sub.label(), "Sub");
    }

    #[test]
    fn default_quality_is_main() {
        assert_eq!(StreamQuality::default(), StreamQuality::Main);
    }

    #[test]
    fn multi_stream_config_defaults() {
        let c = MultiStreamConfig::default();
        assert!(c.sub_stream_url.is_none());
        assert_eq!(c.default_quality, StreamQuality::Main);
    }

    #[test]
    fn multi_stream_config_file_overrides() {
        let f = MultiStreamConfigFile {
            sub_stream_url: Some("rtsp://cam/sub".into()),
            default_quality: Some("sub".into()),
        };
        let c = f.into_config();
        assert_eq!(c.sub_stream_url.as_deref(), Some("rtsp://cam/sub"));
        assert_eq!(c.default_quality, StreamQuality::Sub);
    }

    #[test]
    fn stream_url_for_quality_main() {
        let config = MultiStreamConfig { sub_stream_url: Some("rtsp://sub".into()), ..Default::default() };
        assert_eq!(stream_url_for_quality("rtsp://main", StreamQuality::Main, &config), "rtsp://main");
    }

    #[test]
    fn stream_url_for_quality_sub() {
        let config = MultiStreamConfig { sub_stream_url: Some("rtsp://sub".into()), ..Default::default() };
        assert_eq!(stream_url_for_quality("rtsp://main", StreamQuality::Sub, &config), "rtsp://sub");
    }

    #[test]
    fn stream_url_for_quality_sub_fallback() {
        let config = MultiStreamConfig::default();
        assert_eq!(stream_url_for_quality("rtsp://main", StreamQuality::Sub, &config), "rtsp://main");
    }

    #[test]
    fn has_sub_stream_true() {
        let config = MultiStreamConfig { sub_stream_url: Some("rtsp://sub".into()), ..Default::default() };
        assert!(has_sub_stream(&config));
    }

    #[test]
    fn has_sub_stream_false() {
        let config = MultiStreamConfig::default();
        assert!(!has_sub_stream(&config));
    }
}
