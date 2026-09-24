use iced::{Color, Shadow, Vector};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    /// Native look for COSMIC / Wayland desktops — graphite surfaces,
    /// hairline borders, soft shadows, rounded corners. This is the default.
    Cosmic,
    Dark,
    Light,
    Amoled,
    OpenCode,
}

#[derive(Clone, Copy)]
pub struct ThemeColors {
    pub background: &'static str,
    pub surface: &'static str,
    pub surface_hover: &'static str,
    /// Slightly lifted surface for popovers, menus and floating panels.
    pub surface_elevated: &'static str,
    pub border: &'static str,
    pub border_strong: &'static str,
    pub text: &'static str,
    pub text_secondary: &'static str,
    pub text_tertiary: &'static str,
    /// Readable text/glyph color when painted on top of `accent_blue`.
    pub on_accent: &'static str,
    pub accent_blue: &'static str,
    pub accent_green: &'static str,
    pub accent_red: &'static str,
    pub accent_amber: &'static str,
    pub status_live: &'static str,
    pub status_offline: &'static str,
    pub status_reconnecting: &'static str,
    pub status_disabled: &'static str,
}

impl Theme {
    /// Global corner radii. COSMIC leans on a consistent rounding scale
    /// rather than per-widget values, so these are shared across themes.
    pub const RADIUS_SM: f32 = 6.0;
    pub const RADIUS_MD: f32 = 10.0;
    pub const RADIUS_LG: f32 = 14.0;

    /// Type scale. One ramp for the whole app so `size(8)` / `size(14)` magic
    /// numbers stop drifting apart.
    pub const TEXT_CAPTION: u16 = 11;
    pub const TEXT_BODY: u16 = 12;
    pub const TEXT_EMPHASIS: u16 = 13;
    pub const TEXT_TITLE: u16 = 15;

    /// Spacing scale (px). Use for `spacing(..)` / `padding(..)` instead of ad-hoc values.
    pub const SPACE_1: f32 = 4.0;
    pub const SPACE_2: f32 = 8.0;
    pub const SPACE_3: f32 = 12.0;
    pub const SPACE_4: f32 = 16.0;

    /// Translucent black used behind on-video captions and action rows.
    pub fn overlay_scrim() -> Color {
        Color::from_rgba(0.0, 0.0, 0.0, 0.55)
    }

    pub fn colors(&self) -> ThemeColors {
        match self {
            Theme::Cosmic => ThemeColors {
                background: "#131316",
                surface: "#1b1b20",
                surface_hover: "#26262d",
                surface_elevated: "#232329",
                border: "#ffffff14",
                border_strong: "#ffffff2b",
                text: "#e7e7ec",
                text_secondary: "#a2a2ad",
                text_tertiary: "#6c6c77",
                on_accent: "#0a0f1c",
                accent_blue: "#5b9dff",
                accent_green: "#4ade80",
                accent_red: "#f87171",
                accent_amber: "#fbbf24",
                status_live: "#4ade80",
                status_offline: "#6c6c77",
                status_reconnecting: "#fbbf24",
                status_disabled: "#4b4b54",
            },
            Theme::Dark => ThemeColors {
                background: "#0a0a0a",
                surface: "#0d0d0d",
                surface_hover: "#141414",
                surface_elevated: "#161616",
                border: "#222222",
                border_strong: "#333333",
                text: "#d4d4d4",
                text_secondary: "#888888",
                text_tertiary: "#525252",
                on_accent: "#0a0a0a",
                accent_blue: "#60a5fa",
                accent_green: "#22c55e",
                accent_red: "#ef4444",
                accent_amber: "#f59e0b",
                status_live: "#33d17a",
                status_offline: "#808080",
                status_reconnecting: "#e5a50a",
                status_disabled: "#5a5a5a",
            },
            Theme::Light => ThemeColors {
                background: "#ffffff",
                surface: "#f5f5f5",
                surface_hover: "#e8e8e8",
                surface_elevated: "#ffffff",
                border: "#dddddd",
                border_strong: "#bbbbbb",
                text: "#1a1a1a",
                text_secondary: "#555555",
                text_tertiary: "#999999",
                on_accent: "#ffffff",
                accent_blue: "#2563eb",
                accent_green: "#16a34a",
                accent_red: "#dc2626",
                accent_amber: "#d97706",
                status_live: "#22c55e",
                status_offline: "#9ca3af",
                status_reconnecting: "#f59e0b",
                status_disabled: "#6b7280",
            },
            Theme::Amoled => ThemeColors {
                background: "#000000",
                surface: "#000000",
                surface_hover: "#111111",
                surface_elevated: "#0b0b0b",
                border: "#111111",
                border_strong: "#222222",
                text: "#d4d4d4",
                text_secondary: "#888888",
                text_tertiary: "#525252",
                on_accent: "#000000",
                accent_blue: "#60a5fa",
                accent_green: "#22c55e",
                accent_red: "#ef4444",
                accent_amber: "#f59e0b",
                status_live: "#33d17a",
                status_offline: "#808080",
                status_reconnecting: "#e5a50a",
                status_disabled: "#5a5a5a",
            },
            Theme::OpenCode => ThemeColors {
                background: "#212121",
                surface: "#252525",
                surface_hover: "#303030",
                surface_elevated: "#2b2b2b",
                border: "#4b4c5c",
                border_strong: "#5c9cf5",
                text: "#e0e0e0",
                text_secondary: "#6a6a6a",
                text_tertiary: "#4b4c5c",
                on_accent: "#10131a",
                accent_blue: "#5c9cf5",
                accent_green: "#7fd88f",
                accent_red: "#e06c75",
                accent_amber: "#f5a742",
                status_live: "#7fd88f",
                status_offline: "#6a6a6a",
                status_reconnecting: "#f5a742",
                status_disabled: "#4b4c5c",
            },
        }
    }

    /// Parse `#rgb`-family hex. Accepts `#rrggbb` and `#rrggbbaa` (with or
    /// without the leading `#`), so themes can express hairline borders as
    /// translucent white/black without a second code path.
    pub fn color_from_hex(hex: &str) -> Color {
        let hex = hex.trim_start_matches('#');
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0) as f32 / 255.0;
        if hex.len() >= 8 {
            Color::from_rgba(byte(0), byte(2), byte(4), byte(6))
        } else if hex.len() >= 6 {
            Color::from_rgb(byte(0), byte(2), byte(4))
        } else {
            Color::BLACK
        }
    }

    /// Soft downward shadow used by floating chrome (toolbar, popovers).
    pub fn drop_shadow(&self) -> Shadow {
        Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: Vector::new(0.0, 2.0),
            blur_radius: 12.0,
        }
    }

    /// Palette handed to `iced::Theme` so stock widgets (pick_list dropdown,
    /// scrollbars, text-input caret, togglers) match the hand-painted chrome
    /// instead of falling back to iced's built-in light theme.
    pub fn palette(&self) -> iced::theme::Palette {
        let c = self.colors();
        iced::theme::Palette {
            background: Self::color_from_hex(c.background),
            text: Self::color_from_hex(c.text),
            primary: Self::color_from_hex(c.accent_blue),
            success: Self::color_from_hex(c.accent_green),
            danger: Self::color_from_hex(c.accent_red),
        }
    }

    /// The corresponding `iced::Theme` for the application-level `.theme()` hook.
    pub fn to_iced(&self) -> iced::Theme {
        match self {
            Theme::Light => iced::Theme::custom("RRV Light".to_string(), self.palette()),
            _ => iced::Theme::custom("RRV".to_string(), self.palette()),
        }
    }

    pub fn all() -> &'static [Theme] {
        &[
            Theme::Cosmic,
            Theme::Dark,
            Theme::Light,
            Theme::Amoled,
            Theme::OpenCode,
        ]
    }
}

impl fmt::Display for Theme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Theme::Cosmic => write!(f, "Cosmic"),
            Theme::Dark => write!(f, "Dark"),
            Theme::Light => write!(f, "Light"),
            Theme::Amoled => write!(f, "AMOLED"),
            Theme::OpenCode => write!(f, "OpenCode"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_colors() {
        let colors = Theme::Dark.colors();
        assert_eq!(colors.background, "#0a0a0a");
        assert_eq!(colors.text, "#d4d4d4");
    }

    #[test]
    fn light_theme_colors() {
        let colors = Theme::Light.colors();
        assert_eq!(colors.background, "#ffffff");
        assert_eq!(colors.text, "#1a1a1a");
    }

    #[test]
    fn amoled_theme_colors() {
        let colors = Theme::Amoled.colors();
        assert_eq!(colors.background, "#000000");
    }

    #[test]
    fn cosmic_is_first_and_default_ready() {
        assert_eq!(Theme::all()[0], Theme::Cosmic);
        assert_eq!(Theme::Cosmic.to_string(), "Cosmic");
    }

    #[test]
    fn color_from_hex_basic() {
        let c = Theme::color_from_hex("#ff0000");
        assert!((c.r - 1.0).abs() < 0.01);
        assert!((c.g - 0.0).abs() < 0.01);
        assert!((c.b - 0.0).abs() < 0.01);
    }

    #[test]
    fn color_from_hex_without_hash() {
        let c = Theme::color_from_hex("00ff00");
        assert!((c.r - 0.0).abs() < 0.01);
        assert!((c.g - 1.0).abs() < 0.01);
        assert!((c.b - 0.0).abs() < 0.01);
    }

    #[test]
    fn color_from_hex_with_alpha() {
        let c = Theme::color_from_hex("#ffffff14");
        assert!((c.r - 1.0).abs() < 0.01);
        assert!((c.a - 0.078).abs() < 0.02);
    }

    #[test]
    fn all_themes_returns_five() {
        assert_eq!(Theme::all().len(), 5);
    }
}
