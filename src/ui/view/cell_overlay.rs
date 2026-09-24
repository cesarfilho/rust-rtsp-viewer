//! Overlays painted on top of a video tile: the on-hover / on-select action
//! row, the name caption, and the unified placeholder for a tile that has no
//! live picture (offline / connecting / paused / disabled).
//!
//! These used to be a full-width bar above the grid (`quick_actions.rs`); the
//! redesign moves them onto the tile itself so the video area never reflows.

use iced::{Element, Length};
use iced::widget::{button, container, row, text};

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
fn action_btn<'a>(glyph: &'a str, msg: Message, active: bool, danger: bool) -> Element<'a, Message> {
    let fg = if danger {
        iced::Color::from_rgb(1.0, 0.45, 0.45)
    } else if active {
        iced::Color::from_rgb(0.55, 0.75, 1.0)
    } else {
        iced::Color::from_rgb(0.92, 0.92, 0.94)
    };
    button(text(glyph).size(13).color(fg))
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

    let mut r = row![].spacing(2);
    if selected {
        r = r
            .push(action_btn("\u{25C9}", Message::Snapshot, false, false))
            .push(action_btn(
                "\u{25CF}",
                Message::ToggleRecording,
                is_recording,
                is_recording,
            ))
            .push(action_btn("\u{266A}", Message::ToggleAudio, is_audio, false));
    }
    r = r.push(action_btn("\u{25A3}", Message::EnterSpotlight(idx), false, false));

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
            container(text(glyph).size(20).color(tint))
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
