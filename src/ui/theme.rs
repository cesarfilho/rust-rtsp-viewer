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
    pub const TEXT_CAPTION: f32 = 11.0;
    pub const TEXT_BODY: f32 = 12.0;
    pub const TEXT_EMPHASIS: f32 = 13.0;
    pub const TEXT_TITLE: f32 = 15.0;

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
                text_tertiary: "#777783",
                on_accent: "#0a0f1c",
                accent_blue: "#5b9dff",
                accent_green: "#4ade80",
                accent_red: "#f87171",
                accent_amber: "#fbbf24",
                status_live: "#4ade80",
                status_offline: "#777783",
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
                text_tertiary: "#707070",
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
                text_tertiary: "#707070",
                on_accent: "#ffffff",
                accent_blue: "#2563eb",
                accent_green: "#15803d",
                accent_red: "#c81e1e",
                accent_amber: "#b45309",
                status_live: "#15803d",
                status_offline: "#6b7280",
                status_reconnecting: "#b45309",
                status_disabled: "#9ca3af",
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
                text_tertiary: "#707070",
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
                border_strong: "#70718a",
                text: "#e0e0e0",
                text_secondary: "#a0a0ab",
                text_tertiary: "#858592",
                on_accent: "#10131a",
                accent_blue: "#5c9cf5",
                accent_green: "#7fd88f",
                accent_red: "#e5757e",
                accent_amber: "#f5a742",
                status_live: "#7fd88f",
                status_offline: "#8a8a95",
                status_reconnecting: "#f5a742",
                status_disabled: "#5f6070",
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

    /// WCAG relative luminance of an sRGB colour (alpha ignored).
    fn luminance(c: Color) -> f32 {
        let lin = |v: f32| {
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    /// WCAG contrast ratio between two colours (1.0 – 21.0).
    pub fn contrast_ratio(a: Color, b: Color) -> f32 {
        let (la, lb) = (Self::luminance(a), Self::luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// Black or white, whichever reads better on `bg`. For text/glyphs on a
    /// status-coloured badge, whose fill changes with the theme (a fixed
    /// white-on-red is fine on Light but only ~2.8:1 on Cosmic's soft red).
    pub fn readable_on(bg: Color) -> Color {
        if Self::contrast_ratio(Color::BLACK, bg) >= Self::contrast_ratio(Color::WHITE, bg) {
            Color::BLACK
        } else {
            Color::WHITE
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
            warning: Self::color_from_hex(c.accent_amber),
            danger: Self::color_from_hex(c.accent_red),
        }
    }

    /// The corresponding `iced::Theme` for the application-level `.theme()` hook.
    pub fn to_iced(&self) -> iced::Theme {
        match self {
            Theme::Light => iced::Theme::custom("RRV Light".to_string(), self.palette()), // i18n-ok: nome próprio
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
            Theme::Cosmic => write!(f, "Cosmic"), // i18n-ok: nome próprio
            Theme::Dark => write!(f, "Dark"),     // i18n-ok: nome próprio
            Theme::Light => write!(f, "Light"),   // i18n-ok: nome próprio
            Theme::Amoled => write!(f, "AMOLED"), // i18n-ok: nome próprio
            Theme::OpenCode => write!(f, "OpenCode"), // i18n-ok: nome próprio
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
        assert_eq!(Theme::Cosmic.to_string(), "Cosmic"); // i18n-ok: nome próprio
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

    fn ratio(a: &str, b: &str) -> f32 {
        Theme::contrast_ratio(Theme::color_from_hex(a), Theme::color_from_hex(b))
    }

    /// Every theme must keep text and accents legible on the surfaces they are
    /// painted on. Thresholds: body/secondary text and accents 4.5:1 (WCAG AA),
    /// tertiary hints 3.3:1, status pips 3:1.
    #[test]
    fn every_theme_meets_contrast_targets() {
        for &t in Theme::all() {
            let c = t.colors();
            let surfaces = [c.background, c.surface, c.surface_elevated, c.surface_hover];
            for bg in surfaces {
                assert!(ratio(c.text, bg) >= 7.0, "{t}: text on {bg}");
                assert!(ratio(c.text_secondary, bg) >= 4.5, "{t}: secondary on {bg}");
            }
            for bg in [c.background, c.surface, c.surface_elevated] {
                assert!(ratio(c.text_tertiary, bg) >= 3.3, "{t}: tertiary on {bg}");
            }
            for bg in [c.surface, c.surface_elevated] {
                for (name, fg) in [
                    ("blue", c.accent_blue),
                    ("green", c.accent_green),
                    ("red", c.accent_red),
                    ("amber", c.accent_amber),
                ] {
                    assert!(ratio(fg, bg) >= 4.5, "{t}: accent {name} on {bg}");
                }
                for (name, fg) in [
                    ("live", c.status_live),
                    ("offline", c.status_offline),
                    ("reconnecting", c.status_reconnecting),
                ] {
                    assert!(ratio(fg, bg) >= 3.0, "{t}: status {name} on {bg}");
                }
            }
            assert!(ratio(c.on_accent, c.accent_blue) >= 4.5, "{t}: on_accent");
        }
    }

    #[test]
    fn readable_on_picks_the_higher_contrast_ink() {
        assert_eq!(Theme::readable_on(Color::BLACK), Color::WHITE);
        assert_eq!(Theme::readable_on(Color::WHITE), Color::BLACK);
        // Cosmic's soft red: dark ink wins over white.
        let red = Theme::color_from_hex(Theme::Cosmic.colors().accent_red);
        assert_eq!(Theme::readable_on(red), Color::BLACK);
    }

    /// Composição de `fg` (com alfa) sobre `bg` opaco, como o renderizador faz.
    fn over(fg: Color, bg: Color) -> Color {
        Color::from_rgb(
            fg.r * fg.a + bg.r * (1.0 - fg.a),
            fg.g * fg.a + bg.g * (1.0 - fg.a),
            fg.b * fg.a + bg.b * (1.0 - fg.a),
        )
    }

    /// Os estados da janela com o daemon (spec `ux-daemon.md`, critério 9):
    /// o texto do banner sobre o tom translúcido e o botão perigoso das
    /// confirmações, em todos os temas e em todos os estados do botão.
    #[test]
    fn the_daemon_states_meet_contrast_targets() {
        for &t in Theme::all() {
            let c = t.colors();
            let hex = Theme::color_from_hex;
            // O banner (alfa 0,16 do tom) fica sobre o fundo da janela; as
            // confirmações, sobre a superfície elevada.
            for (name, tone) in [("amber", c.accent_amber), ("red", c.accent_red)] {
                for base in [c.background, c.surface] {
                    let tinted = over(
                        Color {
                            a: 0.16,
                            ..hex(tone)
                        },
                        hex(base),
                    );
                    let text = Theme::contrast_ratio(hex(c.text), tinted);
                    assert!(
                        text >= 7.0,
                        "{t}: texto do banner {name} sobre {base}: {text:.1}"
                    );
                    // Os botões do banner usam o mesmo texto principal (`view::daemon`).
                }
            }
            // O botão perigoso: preto ou branco sobre o vermelho, e nos estados
            // hover (clareia 10%) e pressed (escurece 8%).
            let red = hex(c.accent_red);
            let ink = Theme::readable_on(red);
            let mix = |a: Color, b: Color, k: f32| {
                Color::from_rgb(
                    a.r + (b.r - a.r) * k,
                    a.g + (b.g - a.g) * k,
                    a.b + (b.b - a.b) * k,
                )
            };
            for (state, bg) in [
                ("normal", red),
                ("hover", mix(red, Color::WHITE, 0.10)),
                ("pressed", mix(red, Color::BLACK, 0.08)),
            ] {
                let r = Theme::contrast_ratio(ink, bg);
                assert!(r >= 4.5, "{t}: botão perigoso ({state}): {r:.1}");
            }
        }
    }
}
