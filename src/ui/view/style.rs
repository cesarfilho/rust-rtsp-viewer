//! Shared, theme-driven styling for the app chrome.
//!
//! Keeping these in one place is what makes the toolbar, quick-actions bar,
//! status bar, popovers and grid tiles read as one surface instead of a stack
//! of independently-bordered boxes — the look COSMIC/Wayland apps default to.

use iced::widget::{button, container};
use iced::{Background, Border, Color, Shadow, Vector};

use super::super::theme::Theme;

pub fn hex(s: &str) -> Color {
    Theme::color_from_hex(s)
}

/// Same as [`hex`] but overrides the alpha channel (0..1).
pub fn hex_a(s: &str, a: f32) -> Color {
    Color { a, ..hex(s) }
}

/// Mix `a` toward `b` by `t` (0..1). Used for subtle hover/press tints.
fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

/// A bar that floats *over* the video rather than sitting in a solid slab:
/// near-opaque surface, hairline all round, soft drop shadow. Used by the
/// toolbar and the immersive-mode reveal rail.
pub fn floating_bar(theme: Theme) -> impl Fn(&iced::Theme) -> container::Style {
    let c = theme.colors();
    let bg = hex(c.surface);
    let bg = Color { a: 0.94, ..bg };
    let line = hex(c.border);
    let sh = theme.drop_shadow();
    move |_| container::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: line,
            width: 1.0,
            radius: 0.0.into(),
        },
        shadow: sh,
        ..container::Style::default()
    }
}

/// A small pill used for status readouts and segmented filters. `active` gives
/// it the tinted accent fill; otherwise it's a quiet surface chip.
pub fn chip(theme: Theme, active: bool) -> impl Fn(&iced::Theme) -> container::Style {
    let c = theme.colors();
    let accent = hex(c.accent_blue);
    let surface = hex(c.surface_hover);
    move |_| container::Style {
        background: Some(Background::Color(if active {
            Color { a: 0.16, ..accent }
        } else {
            Color { a: 0.6, ..surface }
        })),
        border: Border {
            color: if active { accent } else { Color::TRANSPARENT },
            width: if active { 1.0 } else { 0.0 },
            radius: Theme::RADIUS_SM.into(),
        },
        ..container::Style::default()
    }
}

/// Bottom-anchored caption scrim for a video tile: a solid translucent band.
/// (iced 0.13 gradients need a `Radians` angle + stops; a flat scrim reads
/// just as cleanly and is cheaper.)
pub fn caption_scrim() -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.5))),
        ..container::Style::default()
    }
}

/// The vertical sidebar panel — surface fill, hairline separating it from the
/// video area.
pub fn sidebar_panel(theme: Theme) -> impl Fn(&iced::Theme) -> container::Style {
    let c = theme.colors();
    let bg = hex(c.surface);
    let line = hex(c.border);
    move |_| container::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: line,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

/// A popover / menu / modal card: elevated surface, hairline border, large
/// radius, pronounced shadow so it clearly floats above the content.
pub fn popover(theme: Theme) -> impl Fn(&iced::Theme) -> container::Style {
    let c = theme.colors();
    let bg = hex(c.surface_elevated);
    let line = hex(c.border_strong);
    move |_| container::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: line,
            width: 1.0,
            radius: Theme::RADIUS_LG.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 32.0,
        },
        ..container::Style::default()
    }
}

/// Button intent, mapped to a fill + text color pair. `Primary`/`Danger` are
/// part of the vocabulary even when no current call site uses them.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub enum Intent {
    /// Low-emphasis: transparent until hovered.
    Ghost,
    /// Selected / toggled-on state: tinted fill, accent text.
    Selected,
    /// High-emphasis call to action: solid accent fill.
    Primary,
    /// Destructive / active-recording: solid red fill.
    Danger,
}

/// A rounded, hover-aware chrome button. One styler for every bar button so
/// they share radius, padding rhythm and feedback.
pub fn pill(theme: Theme, intent: Intent) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let c = theme.colors();
    let surface = hex(c.surface_hover);
    let accent = hex(c.accent_blue);
    let danger = hex(c.accent_red);
    let text = hex(c.text);
    let text_dim = hex(c.text_secondary);
    let on_accent = hex(c.on_accent);

    move |_, status| {
        let (base_bg, base_fg): (Color, Color) = match intent {
            Intent::Ghost => (Color::TRANSPARENT, text_dim),
            Intent::Selected => (
                Color {
                    a: 0.16,
                    ..accent
                },
                accent,
            ),
            Intent::Primary => (accent, on_accent),
            Intent::Danger => (danger, Color::WHITE),
        };
        let bg = match status {
            button::Status::Active | button::Status::Disabled => base_bg,
            button::Status::Hovered => match intent {
                Intent::Ghost => Color { a: 0.10, ..text },
                Intent::Selected => Color { a: 0.24, ..accent },
                Intent::Primary => mix(accent, Color::WHITE, 0.10),
                Intent::Danger => mix(danger, Color::WHITE, 0.10),
            },
            button::Status::Pressed => match intent {
                Intent::Ghost => Color { a: 0.16, ..surface },
                Intent::Selected => Color { a: 0.30, ..accent },
                Intent::Primary => mix(accent, Color::BLACK, 0.08),
                Intent::Danger => mix(danger, Color::BLACK, 0.08),
            },
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: base_fg,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: Theme::RADIUS_SM.into(),
            },
            ..button::Style::default()
        }
    }
}

/// A menu row inside a popover — full-width ghost button with its own hover
/// fill and matching radius.
pub fn menu_item(
    theme: Theme,
    danger: bool,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let c = theme.colors();
    let text = if danger {
        hex(c.accent_red)
    } else {
        hex(c.text)
    };
    let accent = hex(c.accent_blue);
    move |_, status| {
        let bg = match status {
            button::Status::Hovered => Color { a: 0.08, ..accent },
            button::Status::Pressed => Color { a: 0.14, ..accent },
            _ => Color::TRANSPARENT,
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: text,
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: Theme::RADIUS_SM.into(),
            },
            ..button::Style::default()
        }
    }
}

/// A keyboard-shortcut chip shown at the right edge of a menu / help row:
/// quiet surface fill, dim text, small radius.
pub fn key_chip(theme: Theme) -> impl Fn(&iced::Theme) -> container::Style {
    let c = theme.colors();
    let bg = hex_a(c.text, 0.06);
    let line = hex(c.border);
    move |_| container::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: line,
            width: 1.0,
            radius: (Theme::RADIUS_SM - 2.0).into(),
        },
        ..container::Style::default()
    }
}

/// A content card inside a panel: barely-lifted surface, hairline, medium
/// radius. Groups related metrics without shouting borders.
pub fn card(theme: Theme) -> impl Fn(&iced::Theme) -> container::Style {
    let c = theme.colors();
    let bg = hex_a(c.text, 0.03);
    let line = hex(c.border);
    move |_| container::Style {
        background: Some(Background::Color(bg)),
        border: Border {
            color: line,
            width: 1.0,
            radius: Theme::RADIUS_MD.into(),
        },
        ..container::Style::default()
    }
}

/// One segment of a segmented control (density picker, layout toggle, group
/// scope). `active` gets the accent tint; the row shares one rounded frame.
pub fn segment(theme: Theme, active: bool) -> impl Fn(&iced::Theme, button::Status) -> button::Style {
    let c = theme.colors();
    let accent = hex(c.accent_blue);
    let dim = hex(c.text_secondary);
    move |_, status| {
        let bg = if active {
            Color { a: 0.18, ..accent }
        } else {
            match status {
                button::Status::Hovered => Color { a: 0.07, ..accent },
                button::Status::Pressed => Color { a: 0.12, ..accent },
                _ => Color::TRANSPARENT,
            }
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: if active { accent } else { dim },
            border: Border {
                color: Color::TRANSPARENT,
                width: 0.0,
                radius: (Theme::RADIUS_SM - 1.0).into(),
            },
            ..button::Style::default()
        }
    }
}
