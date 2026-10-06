//! Overlays painted on top of a video tile: the on-hover / on-select action
//! row, the name caption, and the unified placeholder for a tile that has no
//! live picture (offline / connecting / paused / disabled).
//!
//! These used to be a full-width bar above the grid (`quick_actions.rs`); the
//! redesign moves them onto the tile itself so the video area never reflows.

use iced::widget::{button, container, row, text};
use iced::{Element, Length};

use super::super::app::App;
use super::super::message::Message;
use super::super::sidebar::CameraStatus;
use super::super::theme::{Theme, ThemeColors};

/// Status colour for the small tile pip / caption dot.
pub fn status_pip_color(colors: ThemeColors, status: &CameraStatus) -> iced::Color {
    match status {
        CameraStatus::Live => Theme::color_from_hex(colors.status_live),
        CameraStatus::Recording => Theme::color_from_hex(colors.accent_red),
        CameraStatus::Reconnecting | CameraStatus::Connecting => {
            Theme::color_from_hex(colors.accent_amber)
        }
        CameraStatus::Offline => Theme::color_from_hex(colors.accent_red),
        CameraStatus::Paused | CameraStatus::Disabled => {
            Theme::color_from_hex(colors.status_disabled)
        }
    }
}

fn pip<'a>(color: iced::Color, size: f32) -> Element<'a, Message> {
    container(text(""))
        .width(size)
        .height(size)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(color)),
            border: iced::Border {
                radius: (size / 2.0).into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// A single icon button in the on-cell action row.
fn action_btn<'a>(
    glyph: &'a str,
    msg: Message,
    active: bool,
    danger: bool,
) -> Element<'a, Message> {
    let fg = if danger {
        iced::Color::from_rgb(1.0, 0.45, 0.45)
    } else if active {
        iced::Color::from_rgb(0.55, 0.75, 1.0)
    } else {
        iced::Color::from_rgb(0.92, 0.92, 0.94)
    };
    button(text(glyph).font(crate::ui::icons::FONT).size(13).color(fg))
        .padding(iced::Padding::from([3, 6]))
        .on_press(msg)
        .style(move |_, status| button::Style {
            background: Some(iced::Background::Color(match status {
                button::Status::Hovered | button::Status::Pressed => {
                    iced::Color::from_rgba(1.0, 1.0, 1.0, 0.14)
                }
                _ => iced::Color::TRANSPARENT,
            })),
            text_color: fg,
            border: iced::Border {
                radius: Theme::RADIUS_SM.into(),
                ..iced::Border::default()
            },
            ..button::Style::default()
        })
        .into()
}

/// The icon actions of a grid tile. Each icon is only a glyph, so each gets a tooltip naming
/// the action and its shortcut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellAction {
    Snapshot,
    Record,
    Audio,
    Spotlight,
}

impl CellAction {
    /// The tooltip text. `active` is the toggle state (recording / audible): the text names
    /// what pressing the icon will do *now*.
    pub fn tooltip(self, active: bool) -> &'static str {
        match (self, active) {
            (Self::Snapshot, _) => "Capturar imagem  (s)",
            (Self::Record, false) => "Gravar  (r)",
            (Self::Record, true) => "Parar gravação  (r)",
            (Self::Audio, false) => "Ouvir áudio  (m)",
            (Self::Audio, true) => "Silenciar  (m)",
            (Self::Spotlight, _) => "Ampliar câmera  (f)",
        }
    }
}

/// `button` with a tooltip `tip` shown on `position` of it.
fn with_tip<'a>(
    button: Element<'a, Message>,
    tip: &'static str,
    position: iced::widget::tooltip::Position,
) -> Element<'a, Message> {
    iced::widget::tooltip(
        button,
        container(text(tip).size(Theme::TEXT_CAPTION))
            .padding(iced::Padding::from([3, 8]))
            .style(container::rounded_box),
        position,
    )
    .gap(6)
    .into()
}

/// The action row for a tile. `selected` tiles get the full set
/// (snapshot / record / audio / spotlight); merely `hovered` tiles get just the
/// spotlight affordance so a hover never implies "this camera is armed".
pub fn cell_actions(app: &App, idx: usize, selected: bool) -> Element<'_, Message> {
    let is_recording = app
        .sidebar
        .cameras
        .get(idx)
        .map(|c| c.status == CameraStatus::Recording)
        .unwrap_or(false);
    let is_audio = app
        .audio_states
        .get(idx)
        .map(|s| s.is_audible())
        .unwrap_or(false);

    // The row sits at the top of the tile: the tooltip goes *below* the icon (above it would be
    // cut off by the window edge on the first row of the grid).
    let below = iced::widget::tooltip::Position::Bottom;
    let mut r = row![].spacing(2);
    if selected {
        r = r
            .push(with_tip(
                action_btn("\u{25C9}", Message::Snapshot, false, false),
                CellAction::Snapshot.tooltip(false),
                below,
            ))
            .push(with_tip(
                action_btn(
                    "\u{25CF}",
                    Message::ToggleRecording,
                    is_recording,
                    is_recording,
                ),
                CellAction::Record.tooltip(is_recording),
                below,
            ))
            .push(with_tip(
                action_btn("\u{266A}", Message::ToggleAudio, is_audio, false),
                CellAction::Audio.tooltip(is_audio),
                below,
            ));
    }
    r = r.push(with_tip(
        action_btn("\u{25A3}", Message::EnterSpotlight(idx), false, false),
        CellAction::Spotlight.tooltip(false),
        below,
    ));

    container(r)
        .padding(2)
        .style(|_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(Theme::overlay_scrim())),
            border: iced::Border {
                radius: Theme::RADIUS_SM.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// Every per-camera feature as an icon button, for the spotlight bar: the
/// icons are visible as soon as the camera opens, each with a tooltip naming
/// the action and its shortcut. Toggle state (recording, audio, zones) shows as
/// a highlighted button.
pub fn feature_actions(app: &App, idx: usize) -> Element<'_, Message> {
    let is_recording = app
        .sidebar
        .cameras
        .get(idx)
        .is_some_and(|c| c.status == CameraStatus::Recording);
    let is_audio = app.audio_states.get(idx).is_some_and(|s| s.is_audible());
    let zone_count = app
        .engine
        .zones
        .get(idx)
        .map_or(0, |z| z.zones.iter().filter(|z| z.is_active()).count());
    let editing_zones = app.zone_edit.as_ref().is_some_and(|e| e.camera_idx == idx);

    let zones_tip = match zone_count {
        0 => "Zonas de movimento".to_string(),
        n => format!("Zonas de movimento ({n})"),
    };
    let items: [(&str, String, Message, bool, bool); 4] = [
        (
            "\u{25C9}",
            "Snapshot  (s)".into(),
            Message::Snapshot,
            false,
            false,
        ),
        (
            "\u{25CF}",
            if is_recording {
                "Parar gravação  (r)"
            } else {
                "Gravar  (r)"
            }
            .into(),
            Message::ToggleRecording,
            is_recording,
            is_recording,
        ),
        (
            "\u{266A}",
            if is_audio {
                "Silenciar  (m)"
            } else {
                "Ouvir áudio  (m)"
            }
            .into(),
            Message::ToggleAudio,
            is_audio,
            false,
        ),
        (
            "\u{2B21}",
            zones_tip,
            Message::EditZones(idx),
            zone_count > 0 || editing_zones,
            false,
        ),
    ];

    let mut r = row![].spacing(2);
    for (glyph, tip, msg, active, danger) in items {
        r = r.push(
            iced::widget::tooltip(
                action_btn(glyph, msg, active, danger),
                container(text(tip).size(Theme::TEXT_CAPTION))
                    .padding(iced::Padding::from([3, 8]))
                    .style(container::rounded_box),
                iced::widget::tooltip::Position::Top,
            )
            .gap(6),
        );
    }
    r.into()
}

/// A compact name chip for the bottom-left of a live tile: a status pip, the
/// name, and `· NN fps` only when the tile is selected or hovered. No
/// edge-to-edge band — just a small rounded label.
pub fn name_chip(
    name: String,
    pip_color: iced::Color,
    fps: Option<f64>,
    selected: bool,
) -> Element<'static, Message> {
    let name_color = if selected {
        iced::Color::from_rgb(0.6, 0.78, 1.0)
    } else {
        iced::Color::from_rgb(0.95, 0.95, 0.97)
    };
    let mut r = row![
        pip(pip_color, 5.0),
        text(name).size(Theme::TEXT_CAPTION).color(name_color),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);
    if let Some(f) = fps {
        r = r.push(
            text(format!("· {:.0} fps", f))
                .size(Theme::TEXT_CAPTION - 1)
                .color(iced::Color::from_rgb(0.72, 0.72, 0.76)),
        );
    }
    container(r)
        .padding(iced::Padding::from([2, 7]))
        .style(|_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(Theme::overlay_scrim())),
            border: iced::Border {
                radius: Theme::RADIUS_SM.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// One component for every "no picture" tile state, so the grid and the
/// spotlight view render offline / connecting / paused / disabled identically.
pub fn placeholder_cell(
    colors: ThemeColors,
    name: String,
    status: &CameraStatus,
    detail: Option<String>,
) -> Element<'static, Message> {
    let (glyph, label, tint) = match status {
        CameraStatus::Connecting => (
            "\u{25CC}",
            "Conectando\u{2026}",
            Theme::color_from_hex(colors.accent_amber),
        ),
        CameraStatus::Reconnecting => (
            "\u{25CC}",
            "Reconectando",
            Theme::color_from_hex(colors.accent_amber),
        ),
        CameraStatus::Paused => (
            "\u{25D0}",
            "Pausada",
            iced::Color::from_rgba(0.58, 0.58, 0.63, 0.9),
        ),
        CameraStatus::Disabled => (
            "\u{25CB}",
            "Desativada",
            iced::Color::from_rgba(0.5, 0.5, 0.55, 0.8),
        ),
        _ => (
            "\u{25CF}",
            "Offline",
            iced::Color::from_rgba(0.72, 0.5, 0.5, 0.9),
        ),
    };
    let ring = Theme::color_from_hex(colors.border);
    container(
        iced::widget::column![
            container(
                text(glyph)
                    .font(crate::ui::icons::FONT)
                    .size(20)
                    .color(tint)
            )
            .width(40)
            .height(40)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(move |_: &iced::Theme| container::Style {
                border: iced::Border {
                    color: ring,
                    width: 1.0,
                    radius: 20.0.into(),
                },
                ..container::Style::default()
            }),
            text(name)
                .size(Theme::TEXT_BODY)
                .color(Theme::color_from_hex(colors.text_secondary)),
            text(label)
                .size(Theme::TEXT_CAPTION)
                .color(Theme::color_from_hex(colors.text_tertiary)),
            text(detail.unwrap_or_default())
                .size(Theme::TEXT_CAPTION - 1)
                .color(Theme::color_from_hex(colors.text_tertiary)),
        ]
        .spacing(Theme::SPACE_2)
        .align_x(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::CellAction;

    #[test]
    fn every_tile_icon_has_a_tooltip_with_its_shortcut() {
        // Os atalhos são os de `handle_key`: s (snapshot), r (gravar), m (áudio), f (spotlight).
        for (action, key) in [
            (CellAction::Snapshot, "(s)"),
            (CellAction::Record, "(r)"),
            (CellAction::Audio, "(m)"),
            (CellAction::Spotlight, "(f)"),
        ] {
            for active in [false, true] {
                let tip = action.tooltip(active);
                assert!(!tip.is_empty());
                assert!(
                    tip.ends_with(key),
                    "{action:?}/{active}: {tip:?} deveria terminar em {key}"
                );
            }
        }
    }

    #[test]
    fn the_toggles_say_what_pressing_them_does_now() {
        assert_eq!(CellAction::Record.tooltip(false), "Gravar  (r)");
        assert_eq!(CellAction::Record.tooltip(true), "Parar gravação  (r)");
        assert_eq!(CellAction::Audio.tooltip(false), "Ouvir áudio  (m)");
        assert_eq!(CellAction::Audio.tooltip(true), "Silenciar  (m)");
        // quem não alterna diz sempre o mesmo
        assert_eq!(
            CellAction::Spotlight.tooltip(false),
            CellAction::Spotlight.tooltip(true)
        );
    }
}
