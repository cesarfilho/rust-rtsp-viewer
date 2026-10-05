//! The one command surface. `command_menu` builds the popover contents used by
//! both the toolbar `⋯` button and the right-click menu on a tile — same
//! sections, same visual language. Every row goes through
//! `Message::RunMenuCommand`, which closes the menu before acting.

use iced::widget::{button, column, container, row, text};
use iced::{Element, Length};

use super::super::app::App;
use super::super::message::{GridMode, LayoutMode, Message};
use super::super::sidebar;
use super::super::theme::Theme;
use super::style;
use crate::domain::view;

pub const MENU_WIDTH: f32 = 248.0;

fn run(msg: Message) -> Message {
    Message::RunMenuCommand(Box::new(msg))
}

/// `[ icon ] label ............... [ key ]`
fn menu_row<'a>(
    theme: Theme,
    icon: &'a str,
    label: impl text::IntoFragment<'a>,
    key: Option<&'a str>,
    msg: Message,
    danger: bool,
) -> Element<'a, Message> {
    let colors = theme.colors();
    let icon_color = if danger {
        Theme::color_from_hex(colors.accent_red)
    } else {
        Theme::color_from_hex(colors.text_secondary)
    };
    let mut r = row![
        container(
            text(icon)
                .font(crate::ui::icons::FONT)
                .size(Theme::TEXT_BODY)
                .color(icon_color)
        )
        .width(18),
        text(label).size(Theme::TEXT_BODY),
        iced::widget::horizontal_space(),
    ]
    .spacing(Theme::SPACE_2)
    .align_y(iced::Alignment::Center);
    if let Some(k) = key {
        r = r.push(
            container(
                text(k)
                    .size(Theme::TEXT_CAPTION - 1)
                    .color(Theme::color_from_hex(colors.text_tertiary)),
            )
            .padding(iced::Padding::from([1, 5]))
            .style(style::key_chip(theme)),
        );
    }
    button(r)
        .width(Length::Fill)
        .padding(iced::Padding::from([6, 10]))
        .on_press(run(msg))
        .style(style::menu_item(theme, danger))
        .into()
}

fn section_header(theme: Theme, title: impl Into<String>) -> Element<'static, Message> {
    container(
        text(title.into().to_uppercase())
            .size(Theme::TEXT_CAPTION - 1)
            .color(Theme::color_from_hex(theme.colors().text_tertiary)),
    )
    .padding(iced::Padding {
        top: 6.0,
        right: 10.0,
        bottom: 3.0,
        left: 10.0,
    })
    .into()
}

/// A joined segmented control (text labels), inside one rounded frame.
fn segmented<'a>(theme: Theme, items: Vec<(String, bool, Message)>) -> Element<'a, Message> {
    let mut r = row![].spacing(2);
    for (label, active, msg) in items {
        r = r.push(
            button(
                text(label)
                    .size(Theme::TEXT_CAPTION)
                    .width(Length::Fill)
                    .align_x(iced::Alignment::Center),
            )
            .width(Length::Fill)
            .padding(iced::Padding::from([4, 6]))
            .on_press(run(msg))
            .style(style::segment(theme, active)),
        );
    }
    container(r)
        .padding(2)
        .width(Length::Fill)
        .style(style::chip(theme, false))
        .into()
}

fn density_row(app: &App) -> Element<'_, Message> {
    let mut items = vec![(
        "Auto".to_string(),
        app.view.mode == GridMode::Auto,
        Message::GridModeChanged(GridMode::Auto),
    )];
    for &(c, r) in view::FIXED_PRESETS.iter() {
        let m = GridMode::Fixed { cols: c, rows: r };
        items.push((
            format!("{c}×{r}"),
            app.view.mode == m,
            Message::GridModeChanged(m),
        ));
    }
    segmented(app.theme, items)
}

fn layout_row(app: &App) -> Element<'_, Message> {
    segmented(
        app.theme,
        vec![
            (
                "Grade".into(),
                app.layout_mode == LayoutMode::Grid,
                Message::LayoutModeChanged(LayoutMode::Grid),
            ),
            (
                "Flex".into(),
                app.layout_mode == LayoutMode::Flex,
                Message::LayoutModeChanged(LayoutMode::Flex),
            ),
        ],
    )
}

fn theme_row(app: &App) -> Element<'_, Message> {
    let mut r = row![].spacing(6).align_y(iced::Alignment::Center);
    for &t in Theme::all() {
        let c = t.colors();
        let fill = Theme::color_from_hex(c.background);
        let ring = Theme::color_from_hex(c.accent_blue);
        let active = app.theme == t;
        r = r.push(
            button(text(""))
                .width(20)
                .height(20)
                .on_press(run(Message::ThemeChanged(t)))
                .style(move |_, _| button::Style {
                    background: Some(iced::Background::Color(fill)),
                    border: iced::Border {
                        color: ring,
                        width: if active { 2.0 } else { 1.0 },
                        radius: Theme::RADIUS_SM.into(),
                    },
                    ..button::Style::default()
                }),
        );
    }
    container(r)
        .padding(iced::Padding {
            top: 2.0,
            right: 10.0,
            bottom: 6.0,
            left: 10.0,
        })
        .into()
}

fn hairline(theme: Theme) -> Element<'static, Message> {
    iced::widget::horizontal_rule(1)
        .style(move |_: &iced::Theme| iced::widget::rule::Style {
            color: Theme::color_from_hex(theme.colors().border),
            width: 1,
            radius: 0.0.into(),
            fill_mode: iced::widget::rule::FillMode::Full,
        })
        .into()
}

/// Build the shared command menu. `ctx` is the camera the right-click landed
/// on; when `None` the camera section falls back to the current selection.
pub fn command_menu(app: &App, ctx: Option<usize>) -> Element<'_, Message> {
    let theme = app.theme;
    let immersive = !app.focus.is_normal();

    let mut col = column![
        section_header(theme, "Exibição"),
        container(density_row(app)).padding(iced::Padding::from([0, 10])),
        container(layout_row(app)).padding(iced::Padding {
            top: 3.0,
            right: 10.0,
            bottom: 0.0,
            left: 10.0
        }),
        menu_row(
            theme,
            "\u{25F1}",
            if immersive {
                "Sair do modo imersivo"
            } else {
                "Modo imersivo"
            },
            Some("h"),
            Message::ToggleImmersive,
            false,
        ),
        menu_row(
            theme,
            "\u{25F2}",
            "Tela cheia",
            Some("F11"),
            Message::ToggleFullscreen,
            false
        ),
    ]
    .spacing(1)
    .width(MENU_WIDTH);

    if let Some(idx) = ctx.or(app.sidebar.selected)
        && let Some(cam) = app.sidebar.cameras.get(idx)
    {
        {
            let enabled = app.engine.camera_enabled.get(idx).copied().unwrap_or(true);
            col = col
                .push(hairline(theme))
                .push(section_header(theme, format!("Câmera · {}", cam.name)))
                .push(menu_row(
                    theme,
                    "\u{25A3}",
                    "Spotlight",
                    Some("f"),
                    Message::EnterSpotlight(idx),
                    false,
                ))
                .push(menu_row(
                    theme,
                    "\u{25C9}",
                    "Snapshot",
                    Some("s"),
                    Message::Snapshot,
                    false,
                ))
                .push(menu_row(
                    theme,
                    "\u{25CF}",
                    if cam.status == sidebar::CameraStatus::Recording {
                        "Parar gravação"
                    } else {
                        "Gravar"
                    },
                    Some("r"),
                    Message::ToggleRecording,
                    false,
                ))
                .push(menu_row(
                    theme,
                    "\u{266A}",
                    "Áudio",
                    Some("m"),
                    Message::ToggleAudio,
                    false,
                ))
                .push(menu_row(
                    theme,
                    "\u{2B21}",
                    "Zonas de movimento",
                    None,
                    Message::EditZones(idx),
                    false,
                ))
                .push(menu_row(
                    theme,
                    "\u{25D0}",
                    if enabled { "Desativar" } else { "Ativar" },
                    None,
                    Message::Sidebar(sidebar::Message::CameraToggled(idx, !enabled)),
                    false,
                ));
        }
    }

    col = col
        .push(hairline(theme))
        .push(section_header(theme, "Aparência"))
        .push(theme_row(app))
        .push(hairline(theme))
        .push(menu_row(
            theme,
            "?",
            "Ajuda",
            Some("?"),
            Message::ShowHelp,
            false,
        ))
        .push(menu_row(
            theme,
            "\u{25CF}",
            "Sair",
            Some("Ctrl Q"),
            Message::Quit,
            true,
        ));

    container(col)
        .padding(4)
        .style(style::popover(theme))
        .into()
}
