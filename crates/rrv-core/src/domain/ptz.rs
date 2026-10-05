use serde::Deserialize;

/// Pan/Tilt/Zoom command for ONVIF cameras.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PtzCommand {
    PanLeft,
    PanRight,
    TiltUp,
    TiltDown,
    ZoomIn,
    ZoomOut,
    Stop,
    /// Go to a preset position.
    GoToPreset(u32),
    /// Set current position as a preset.
    SetPreset(u32),
}

impl PtzCommand {
    pub fn label(&self) -> &'static str {
        match self {
            PtzCommand::PanLeft => "Pan Left",
            PtzCommand::PanRight => "Pan Right",
            PtzCommand::TiltUp => "Tilt Up",
            PtzCommand::TiltDown => "Tilt Down",
            PtzCommand::ZoomIn => "Zoom In",
            PtzCommand::ZoomOut => "Zoom Out",
            PtzCommand::Stop => "Stop",
            PtzCommand::GoToPreset(_) => "Go to Preset",
            PtzCommand::SetPreset(_) => "Set Preset",
        }
    }

    pub fn key_binding(&self) -> Option<&'static str> {
        match self {
            PtzCommand::PanLeft => Some("ArrowLeft"),
            PtzCommand::PanRight => Some("ArrowRight"),
            PtzCommand::TiltUp => Some("ArrowUp"),
            PtzCommand::TiltDown => Some("ArrowDown"),
            PtzCommand::ZoomIn => Some("="),
            PtzCommand::ZoomOut => Some("-"),
            PtzCommand::Stop => Some("0"),
            _ => None,
        }
    }
}

/// Configuration for ONVIF PTZ control on a camera.
#[derive(Debug, Clone, Deserialize)]
pub struct PtzConfig {
    /// ONVIF service URL (e.g. "http://192.168.1.100/onvif/device_service").
    pub onvif_url: String,
    /// ONVIF username.
    pub username: Option<String>,
    /// ONVIF password.
    pub password: Option<String>,
    /// Pan/tilt speed 0.0..=1.0.
    pub pan_tilt_speed: f32,
    /// Zoom speed 0.0..=1.0.
    pub zoom_speed: f32,
    /// Available preset names mapped to preset IDs.
    pub presets: Vec<PtzPreset>,
}

impl Default for PtzConfig {
    fn default() -> Self {
        Self {
            onvif_url: String::new(),
            username: None,
            password: None,
            pan_tilt_speed: 0.5,
            zoom_speed: 0.5,
            presets: vec![],
        }
    }
}

/// A named PTZ preset position.
#[derive(Debug, Clone, Deserialize)]
pub struct PtzPreset {
    pub id: u32,
    pub name: String,
}

/// TOML mirror for `[ptz]` section.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct PtzConfigFile {
    pub onvif_url: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub pan_tilt_speed: Option<f32>,
    pub zoom_speed: Option<f32>,
    pub presets: Option<Vec<PtzPresetFile>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct PtzPresetFile {
    pub id: u32,
    pub name: String,
}

impl PtzConfigFile {
    pub fn into_config(self) -> PtzConfig {
        let mut config = PtzConfig::default();
        if let Some(u) = self.onvif_url {
            config.onvif_url = u;
        }
        if let Some(u) = self.username {
            config.username = Some(u);
        }
        if let Some(p) = self.password {
            config.password = Some(p);
        }
        if let Some(s) = self.pan_tilt_speed {
            config.pan_tilt_speed = s.clamp(0.0, 1.0);
        }
        if let Some(s) = self.zoom_speed {
            config.zoom_speed = s.clamp(0.0, 1.0);
        }
        if let Some(presets) = self.presets {
            config.presets = presets
                .into_iter()
                .map(|p| PtzPreset {
                    id: p.id,
                    name: p.name,
                })
                .collect();
        }
        config
    }
}

/// Result of a PTZ operation.
#[derive(Debug, Clone)]
pub struct PtzResult {
    pub success: bool,
    pub error: Option<String>,
}

impl PtzResult {
    pub fn ok() -> Self {
        Self {
            success: true,
            error: None,
        }
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(msg.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ptz_command_labels() {
        assert_eq!(PtzCommand::PanLeft.label(), "Pan Left");
        assert_eq!(PtzCommand::ZoomIn.label(), "Zoom In");
        assert_eq!(PtzCommand::Stop.label(), "Stop");
    }

    #[test]
    fn ptz_command_key_bindings() {
        assert_eq!(PtzCommand::PanLeft.key_binding(), Some("ArrowLeft"));
        assert_eq!(PtzCommand::ZoomIn.key_binding(), Some("="));
        assert_eq!(PtzCommand::GoToPreset(1).key_binding(), None);
    }

    #[test]
    fn ptz_config_defaults() {
        let c = PtzConfig::default();
        assert!(c.onvif_url.is_empty());
        assert_eq!(c.pan_tilt_speed, 0.5);
        assert_eq!(c.zoom_speed, 0.5);
        assert!(c.presets.is_empty());
    }

    #[test]
    fn ptz_config_file_overrides() {
        let f = PtzConfigFile {
            onvif_url: Some("http://cam/onvif".into()),
            username: Some("admin".into()),
            password: Some("pass".into()),
            pan_tilt_speed: Some(0.8),
            zoom_speed: Some(0.3),
            presets: Some(vec![PtzPresetFile {
                id: 1,
                name: "Home".into(),
            }]),
        };
        let c = f.into_config();
        assert_eq!(c.onvif_url, "http://cam/onvif");
        assert_eq!(c.username.as_deref(), Some("admin"));
        assert_eq!(c.password.as_deref(), Some("pass"));
        assert_eq!(c.pan_tilt_speed, 0.8);
        assert_eq!(c.zoom_speed, 0.3);
        assert_eq!(c.presets.len(), 1);
        assert_eq!(c.presets[0].name, "Home");
    }

    #[test]
    fn ptz_result_ok() {
        let r = PtzResult::ok();
        assert!(r.success);
        assert!(r.error.is_none());
    }

    #[test]
    fn ptz_result_err() {
        let r = PtzResult::err("connection failed");
        assert!(!r.success);
        assert_eq!(r.error.as_deref(), Some("connection failed"));
    }
}
