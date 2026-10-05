use serde::Deserialize;

/// Speed multiplier for time-lapse export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeedMultiplier(pub u32);

impl SpeedMultiplier {
    pub fn new(multiplier: u32) -> Self {
        Self(multiplier.clamp(2, 3600))
    }

    pub fn value(&self) -> u32 {
        self.0
    }

    pub fn label(&self) -> String {
        if self.0 >= 3600 {
            format!("{}h", self.0 / 3600)
        } else if self.0 >= 60 {
            format!("{}m", self.0 / 60)
        } else {
            format!("{}x", self.0)
        }
    }
}

impl Default for SpeedMultiplier {
    fn default() -> Self {
        Self(10)
    }
}

/// Output format for time-lapse export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
pub enum TimelapseFormat {
    #[default]
    Mp4,
    WebM,
    Gif,
}

impl TimelapseFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            TimelapseFormat::Mp4 => "mp4",
            TimelapseFormat::WebM => "webm",
            TimelapseFormat::Gif => "gif",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            TimelapseFormat::Mp4 => "MP4",
            TimelapseFormat::WebM => "WebM",
            TimelapseFormat::Gif => "GIF",
        }
    }
}

/// Configuration for time-lapse export.
#[derive(Debug, Clone)]
pub struct TimelapseConfig {
    /// Speed multiplier (e.g. 10 = 10x faster).
    pub speed: SpeedMultiplier,
    /// Output format.
    pub format: TimelapseFormat,
    /// Output directory.
    pub output_dir: std::path::PathBuf,
    /// Output resolution width (None = keep original).
    pub width: Option<u32>,
    /// Output resolution height (None = keep original).
    pub height: Option<u32>,
    /// Frames per second for the output video.
    pub fps: u32,
}

impl Default for TimelapseConfig {
    fn default() -> Self {
        Self {
            speed: SpeedMultiplier::default(),
            format: TimelapseFormat::default(),
            output_dir: default_timelapse_dir(),
            width: None,
            height: None,
            fps: 30,
        }
    }
}

fn default_timelapse_dir() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("XDG_VIDEOS_DIR")
        && !p.is_empty()
    {
        return std::path::PathBuf::from(p)
            .join("rust-rtsp-viewer")
            .join("timelapse");
    }
    if let Ok(home) = std::env::var("HOME") {
        return std::path::PathBuf::from(home)
            .join("Videos")
            .join("rust-rtsp-viewer")
            .join("timelapse");
    }
    std::path::PathBuf::from("./Videos/rust-rtsp-viewer/timelapse")
}

/// TOML mirror for `[timelapse]` section.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct TimelapseConfigFile {
    pub speed: Option<u32>,
    pub format: Option<String>,
    pub output_dir: Option<std::path::PathBuf>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<u32>,
}

impl TimelapseConfigFile {
    pub fn into_config(self) -> TimelapseConfig {
        let mut config = TimelapseConfig::default();
        if let Some(s) = self.speed {
            config.speed = SpeedMultiplier::new(s);
        }
        if let Some(f) = self.format {
            config.format = match f.to_lowercase().as_str() {
                "webm" => TimelapseFormat::WebM,
                "gif" => TimelapseFormat::Gif,
                _ => TimelapseFormat::Mp4,
            };
        }
        if let Some(d) = self.output_dir {
            config.output_dir = d;
        }
        if let Some(w) = self.width {
            config.width = Some(w.max(1));
        }
        if let Some(h) = self.height {
            config.height = Some(h.max(1));
        }
        if let Some(f) = self.fps {
            config.fps = f.clamp(1, 120);
        }
        config
    }
}

/// Estimate the output duration given an input duration in seconds
/// and the speed multiplier.
pub fn estimate_output_duration(input_secs: u64, speed: SpeedMultiplier) -> u64 {
    input_secs / speed.value() as u64
}

/// Generate a filename for the time-lapse output.
pub fn generate_timelapse_filename(camera_label: &str, format: TimelapseFormat) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let (y, mo, d, h, mi, s) = epoch_to_ymdhms(now);
    format!(
        "timelapse-{}-{:04}{:02}{:02}-{:02}{:02}{:02}.{}",
        camera_label,
        y,
        mo,
        d,
        h,
        mi,
        s,
        format.extension()
    )
}

fn epoch_to_ymdhms(epoch: u64) -> (u32, u32, u32, u32, u32, u32) {
    let secs = epoch % 60;
    let total_mins = epoch / 60;
    let mins = total_mins % 60;
    let total_hours = total_mins / 60;
    let hours = total_hours % 24;
    let days = total_hours / 24;

    // Simplified date calculation.
    let mut y = 1970u32;
    let mut remaining_days = days;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining_days < days_in_year as u64 {
            break;
        }
        remaining_days -= days_in_year as u64;
        y += 1;
    }

    let days_in_month = [
        31,
        if is_leap(y) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 1u32;
    for &dim in &days_in_month {
        if remaining_days < dim as u64 {
            break;
        }
        remaining_days -= dim as u64;
        m += 1;
    }

    (
        y,
        m,
        remaining_days as u32 + 1,
        hours as u32,
        mins as u32,
        secs as u32,
    )
}

fn is_leap(y: u32) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_multiplier_clamp() {
        assert_eq!(SpeedMultiplier::new(0).value(), 2);
        assert_eq!(SpeedMultiplier::new(5000).value(), 3600);
        assert_eq!(SpeedMultiplier::new(10).value(), 10);
    }

    #[test]
    fn speed_multiplier_label() {
        assert_eq!(SpeedMultiplier(5).label(), "5x");
        assert_eq!(SpeedMultiplier(60).label(), "1m");
        assert_eq!(SpeedMultiplier(3600).label(), "1h");
    }

    #[test]
    fn format_extension() {
        assert_eq!(TimelapseFormat::Mp4.extension(), "mp4");
        assert_eq!(TimelapseFormat::WebM.extension(), "webm");
        assert_eq!(TimelapseFormat::Gif.extension(), "gif");
    }

    #[test]
    fn test_estimate_output_duration() {
        assert_eq!(estimate_output_duration(100, SpeedMultiplier(10)), 10);
        assert_eq!(estimate_output_duration(3600, SpeedMultiplier(60)), 60);
        assert_eq!(estimate_output_duration(5, SpeedMultiplier(10)), 0);
    }

    #[test]
    fn generate_filename() {
        let f = generate_timelapse_filename("Garagem", TimelapseFormat::Mp4);
        assert!(f.starts_with("timelapse-Garagem-"));
        assert!(f.ends_with(".mp4"));
    }

    #[test]
    fn config_file_overrides() {
        let f = TimelapseConfigFile {
            speed: Some(30),
            format: Some("webm".into()),
            width: Some(1280),
            height: Some(720),
            fps: Some(24),
            ..Default::default()
        };
        let c = f.into_config();
        assert_eq!(c.speed.value(), 30);
        assert_eq!(c.format, TimelapseFormat::WebM);
        assert_eq!(c.width, Some(1280));
        assert_eq!(c.height, Some(720));
        assert_eq!(c.fps, 24);
    }

    #[test]
    fn config_file_clamps() {
        let f = TimelapseConfigFile {
            speed: Some(0),
            fps: Some(200),
            ..Default::default()
        };
        let c = f.into_config();
        assert_eq!(c.speed.value(), 2);
        assert_eq!(c.fps, 120);
    }
}
