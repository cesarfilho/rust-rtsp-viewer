//! The top toolbar, the immersive-mode reveal rail, and the `⋯` overflow menu.
//!
//! Layout is three zones separated by flexible space:
//!   left   — sidebar toggle · Grid/Flex · density
//!   centre — page navigator · carousel   (grid layout only, contextual)
//!   right  — camera-health readout · REC · overflow menu

use iced::{Element, Length};

use super::super::app::App;
use super::super::message::{GridMode, LayoutMode, Message};
use super::super::sidebar::CameraStatus;
use super::super::theme::Theme;
use super::style::{self, Intent};
use crate::domain::view;

/// `(page_1based, page_count)` for the current grid page — mirrors
/// `update::grid_page_*`.
fn page_info(app: &App) -> (usize, usize) {
    let visible = app.sidebar.visible_camera_indices(app.videos.len());
    let ordered = view::apply_order(&app.view.order, &visible);
    let page_size = app.view.mode.page_size(ordered.len());
    let page_count = view::page_count(ordered.len(), page_size);
    (view::clamp_page(app.current_page, page_count) + 1, page_count)
}

/// `(live, total)` and a colour hint for the health dot.
fn health(app: &App) -> (usize, usize, &'static str) {
    let colors = app.theme.colors();
    let total = app.sidebar.cameras.len();
    let live = app
        .sidebar
        .cameras
        .iter()
        .filter(|c| c.status == CameraStatus::Live || c.status == CameraStatus::Recording)
        .count();
    let any_offline = app
        .sidebar
        .cameras
        .iter()
        .any(|c| c.status == CameraStatus::Offline);
    let any_wait = app.sidebar.cameras.iter().any(|c| {
        matches!(
            c.status,
            CameraStatus::Reconnecting | CameraStatus::Connecting
        )
    });
    let color = if total == 0 || any_offline {
        colors.status_offline
    } else if any_wait || live < total {
        colors.accent_amber
    } else {
        colors.status_live
    };
    (live, total, color)
}

fn pill<'a>(
    label: impl Into<Element<'a, Message>>,
    intent: Intent,
    msg: Message,
    theme: Theme,
) -> Element<'a, Message> {
    iced::widget::button(label)
        .on_press(msg)
        .padding([5, 10])
        .style(style::pill(theme, intent))
        .into()
}

/// A joined segmented control of grid-density presets — replaces the stock
/// `pick_list` so it matches the chrome.
fn density_segments(app: &App) -> Element<'_, Message> {
    let theme = app.theme;
    let seg = |label: String, active: bool, msg: Message| {
        iced::widget::button(
            iced::widget::text(label).size(Theme::TEXT_CAPTION),
        )
        .padding(iced::Padding::from([4, 7]))
        .on_press(msg)
        .style(style::segment(theme, active))
    };
    let mut r = iced::widget::row![seg(
        "Auto".into(),
        app.view.mode == GridMode::Auto,
        Message::GridModeChanged(GridMode::Auto)
    )]
    .spacing(1);
    for &(c, rr) in view::FIXED_PRESETS.iter() {
        let m = GridMode::Fixed { cols: c, rows: rr };
        r = r.push(seg(format!("{c}×{rr}"), app.view.mode == m, Message::GridModeChanged(m)));
    }
    iced::widget::container(r)
        .padding(2)
        .style(style::chip(theme, false))
        .into()
}

/// Page navigator + carousel toggle. Shared by the toolbar and the immersive
/// reveal rail. Density lives beside it (added by the caller).
fn view_controls(app: &App, compact: bool) -> Element<'_, Message> {
    let theme = app.theme;
    let colors = theme.colors();
    let dim = Theme::color_from_hex(colors.text_secondary);

    let mut r = iced::widget::row![]
        .spacing(Theme::SPACE_2)
        .align_y(iced::Alignment::Center);

    let (page, page_count) = page_info(app);
    let rotate_on = app.view.rotate_enabled;
    if page_count > 1 || rotate_on {
        let mut nav = iced::widget::row![]
            .spacing(4)
            .align_y(iced::Alignment::Center);
        if page_count > 1 {
            nav = nav
                .push(pill(
                    iced::widget::text("\u{2039}").size(13),
                    Intent::Ghost,
                    Message::PrevPage,
                    theme,
                ))
                .push(
                    iced::widget::text(format!("{page}/{page_count}"))
                        .size(Theme::TEXT_BODY)
                        .color(dim),
                )
                .push(pill(
                    iced::widget::text("\u{203A}").size(13),
                    Intent::Ghost,
                    Message::NextPage,
                    theme,
                ));
        }
        nav = nav.push(pill(
            iced::widget::text(if rotate_on {
                format!("\u{21BB} {}s", app.view.rotate_secs)
            } else {
                "\u{21BB}".to_string()
            })
            .size(Theme::TEXT_BODY),
            if rotate_on { Intent::Selected } else { Intent::Ghost },
            Message::ToggleRotate,
            theme,
        ));
        if rotate_on && !compact {
            nav = nav
                .push(pill(
                    iced::widget::text("\u{2212}").size(12),
                    Intent::Ghost,
                    Message::RotateIntervalStep(false),
                    theme,
                ))
                .push(pill(
                    iced::widget::text("+").size(12),
                    Intent::Ghost,
                    Message::RotateIntervalStep(true),
                    theme,
                ));
        }
        r = r.push(
            iced::widget::container(nav)
                .padding(3)
                .style(style::chip(theme, false)),
        );
    }

    r.into()
}

/// A thin bar split into one segment per camera, coloured by that camera's
/// state, with a quiet `live/total` readout — reads "how many, and which, are
/// up" at a glance.
fn health_meter(app: &App) -> Element<'_, Message> {
    let theme = app.theme;
    let colors = theme.colors();
    let total = app.sidebar.cameras.len();
    if total == 0 {
        return iced::widget::container(
            iced::widget::text("sem câmeras")
                .size(Theme::TEXT_CAPTION)
                .color(Theme::color_from_hex(colors.text_tertiary)),
        )
        .padding([3, 8])
        .style(style::chip(theme, false))
        .into();
    }
    let live = app
        .sidebar
        .cameras
        .iter()
        .filter(|c| matches!(c.status, CameraStatus::Live | CameraStatus::Recording))
        .count();
    let seg_w = (110.0 / total as f32).clamp(3.0, 12.0);

    let mut bar = iced::widget::row![].spacing(1);
    for c in &app.sidebar.cameras {
        let col = match c.status {
            CameraStatus::Live | CameraStatus::Recording => colors.status_live,
            CameraStatus::Reconnecting | CameraStatus::Connecting => colors.accent_amber,
            CameraStatus::Offline => colors.accent_red,
            CameraStatus::Paused | CameraStatus::Disabled => colors.status_disabled,
        };
        bar = bar.push(
            iced::widget::container(iced::widget::text(""))
                .width(seg_w)
                .height(6)
                .style(move |_: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(Theme::color_from_hex(col))),
                    border: iced::Border {
                        radius: 1.0.into(),
                        ..iced::Border::default()
                    },
                    ..iced::widget::container::Style::default()
                }),
        );
    }
    iced::widget::container(
        iced::widget::row![
            bar,
            iced::widget::text(format!("{live}/{total}"))
                .size(Theme::TEXT_CAPTION)
                .color(Theme::color_from_hex(colors.text_secondary)),
        ]
        .spacing(Theme::SPACE_2)
        .align_y(iced::Alignment::Center),
    )
    .padding([4, 8])
    .style(style::chip(theme, false))
    .into()
}

pub fn toolbar(app: &App) -> Element<'_, Message> {
    let theme = app.theme;
    let colors = theme.colors();
    let wide = app.window_size.width >= 820.0;

    let hamburger = pill(
        iced::widget::text("\u{2630}").size(14),
        if app.sidebar.visible {
            Intent::Selected
        } else {
            Intent::Ghost
        },
        Message::ToggleSidebar,
        theme,
    );

    let layout_seg = iced::widget::container(
        iced::widget::row![
            pill(
                iced::widget::text("Grade").size(Theme::TEXT_BODY),
                if app.layout_mode == LayoutMode::Grid {
                    Intent::Selected
                } else {
                    Intent::Ghost
                },
                Message::LayoutModeChanged(LayoutMode::Grid),
                theme,
            ),
            pill(
                iced::widget::text("Flex").size(Theme::TEXT_BODY),
                if app.layout_mode == LayoutMode::Flex {
                    Intent::Selected
                } else {
                    Intent::Ghost
                },
                Message::LayoutModeChanged(LayoutMode::Flex),
                theme,
            ),
        ]
        .spacing(2),
    )
    .padding(2)
    .style(style::chip(theme, false));

    let mut left = iced::widget::row![hamburger, layout_seg]
        .spacing(Theme::SPACE_2)
        .align_y(iced::Alignment::Center);
    if app.layout_mode == LayoutMode::Grid && wide {
        left = left.push(density_segments(app));
        left = left.push(view_controls(app, false));
    }

    // Right cluster: segmented health meter, REC, overflow.
    let health_chip = health_meter(app);

    let rec_badge: Element<'_, Message> = if app.is_recording {
        iced::widget::container(
            iced::widget::text("REC").size(Theme::TEXT_CAPTION).color(iced::Color::WHITE),
        )
        .padding([2, 6])
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(Theme::color_from_hex(colors.accent_red))),
            border: iced::Border {
                radius: 4.0.into(),
                ..iced::Border::default()
            },
            ..iced::widget::container::Style::default()
        })
        .into()
    } else {
        iced::widget::horizontal_space().width(0).into()
    };

    let overflow = pill(
        iced::widget::text("\u{22EF}").size(15),
        if app.show_overflow_menu {
            Intent::Selected
        } else {
            Intent::Ghost
        },
        Message::ToggleOverflowMenu,
        theme,
    );

    let right = iced::widget::row![health_chip, rec_badge, overflow]
        .spacing(Theme::SPACE_2)
        .align_y(iced::Alignment::Center);

    let bar = iced::widget::row![left, iced::widget::horizontal_space(), right]
        .padding([4, Theme::SPACE_3 as u16])
        .spacing(Theme::SPACE_2)
        .align_y(iced::Alignment::Center);

    iced::widget::container(bar)
        .width(Length::Fill)
        .height(TOOLBAR_H)
        .style(style::floating_bar(theme))
        .into()
}

const TOOLBAR_H: f32 = 40.0;

/// The compact control strip that slides in over immersive / spotlight video.
pub fn chrome_rail(app: &App) -> Element<'_, Message> {
    let theme = app.theme;
    let colors = theme.colors();

    let exit = pill(
        iced::widget::text("\u{2922} sair").size(Theme::TEXT_BODY),
        Intent::Ghost,
        Message::ExitFocus,
        theme,
    );
    let (live, total, hc) = health(app);
    let dot = iced::widget::container(iced::widget::text(""))
        .width(7)
        .height(7)
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(Theme::color_from_hex(hc))),
            border: iced::Border {
                radius: 4.0.into(),
                ..iced::Border::default()
            },
            ..iced::widget::container::Style::default()
        });

    let mut r = iced::widget::row![exit]
        .spacing(Theme::SPACE_2)
        .align_y(iced::Alignment::Center);
    if matches!(app.layout_mode, LayoutMode::Grid) && app.focus == super::super::app::ViewFocus::Immersive
    {
        r = r.push(view_controls(app, true));
    }
    if let super::super::app::ViewFocus::Spotlight(_) = app.focus {
        r = r
            .push(pill(
                iced::widget::text("\u{2039}").size(13),
                Intent::Ghost,
                Message::SpotlightStep(false),
                theme,
            ))
            .push(pill(
                iced::widget::text("\u{203A}").size(13),
                Intent::Ghost,
                Message::SpotlightStep(true),
                theme,
            ));
    }
    r = r.push(iced::widget::horizontal_space()).push(
        iced::widget::row![
            dot,
            iced::widget::text(format!("{live}/{total}"))
                .size(Theme::TEXT_BODY)
                .color(Theme::color_from_hex(colors.text_secondary)),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
    );

    iced::widget::container(
        r.padding([5, Theme::SPACE_3 as u16]).align_y(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .style(style::floating_bar(theme))
    .into()
}

/// The `⋯` dropdown — the shared command menu, anchored below the toolbar.
pub fn overflow_menu_layer(app: &App) -> Option<Element<'_, Message>> {
    if !app.show_overflow_menu {
        return None;
    }
    Some(
        iced::widget::container(super::menu::command_menu(app, None))
            .align_top(TOOLBAR_H as u16 + 4)
            .align_right(8.0)
            .into(),
    )
}
