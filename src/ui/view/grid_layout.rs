use iced::alignment::{Horizontal, Vertical};
use iced::{Element, Length};

use super::super::app::App;
use super::super::grid;
use super::super::message::Message;
use super::super::sidebar::CameraStatus;
use super::super::theme::Theme;
use super::cell_overlay;
use crate::domain::view;

const GAP: f32 = 3.0;

/// Wrap a tile so it selects on click, opens the per-camera menu on
/// right-click, and reports hover so the on-cell action row can appear.
fn clickable<'a>(cell: impl Into<Element<'a, Message>>, idx: usize) -> Element<'a, Message> {
    iced::widget::mouse_area(cell)
        .on_press(Message::SelectCamera(idx))
        .on_right_press(Message::ShowContextMenu(idx))
        .on_enter(Message::CellHoverEnter(idx))
        .on_exit(Message::CellHoverExit)
        .into()
}

fn small_badge<'a>(label: String, bg: iced::Color, fg: iced::Color) -> Element<'a, Message> {
    iced::widget::container(
        iced::widget::text(label)
            .font(crate::ui::icons::FONT)
            .size(10)
            .color(fg),
    )
    .padding(iced::Padding::from([1, 4]))
    .style(move |_: &iced::Theme| iced::widget::container::Style {
        background: Some(iced::Background::Color(bg)),
        border: iced::Border {
            radius: 3.0.into(),
            ..iced::Border::default()
        },
        ..iced::widget::container::Style::default()
    })
    .into()
}

pub fn grid_layout(app: &App, sidebar_width: f32, available_height: f32) -> Element<'_, Message> {
    let colors = app.theme.colors();
    let border_color = Theme::color_from_hex(colors.border);
    let slot_bg = Theme::color_from_hex(colors.surface);
    let video_bg = iced::Color::from_rgb(0.0, 0.0, 0.0);
    let gutter_bg = Theme::color_from_hex(colors.background);
    let tile_radius: iced::border::Radius = Theme::RADIUS_MD.into();

    // Group filter → display order → page slice.
    let visible = app.sidebar.visible_camera_indices(app.videos.len());
    let ordered = view::apply_order(&app.view.order, &visible);
    let page_size = app.view.mode.page_size(ordered.len());
    let page_count = view::page_count(ordered.len(), page_size);
    let page = view::clamp_page(app.current_page, page_count);
    let slice: Vec<usize> = ordered[view::page_slice(page, page_size, ordered.len())].to_vec();

    let grid_area_width = (app.window_size.width - sidebar_width).max(1.0);
    let is_fixed = app.view.mode.dims().is_some();
    let (cols, rows) = match app.view.mode.dims() {
        Some((c, r)) => (c.max(1), r.max(1)),
        None => {
            let gi = grid::calc_grid(slice.len(), grid_area_width, available_height);
            (gi.cols.max(1), gi.rows.max(1))
        }
    };

    let mut grid_rows = iced::widget::column![].spacing(GAP);
    for row in 0..rows {
        let mut row_widgets = iced::widget::row![].spacing(GAP);
        for col in 0..cols {
            let slot = row * cols + col;
            if slot >= slice.len() {
                if is_fixed {
                    row_widgets = row_widgets.push(
                        iced::widget::container(iced::widget::text(""))
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .style(move |_: &iced::Theme| iced::widget::container::Style {
                                background: Some(iced::Background::Color(slot_bg)),
                                border: iced::Border {
                                    color: border_color,
                                    width: 1.0,
                                    radius: tile_radius,
                                },
                                ..iced::widget::container::Style::default()
                            }),
                    );
                }
                continue;
            }

            let idx = slice[slot];
            let status = app
                .sidebar
                .cameras
                .get(idx)
                .map(|c| c.status.clone())
                .unwrap_or(CameraStatus::Offline);
            let name = app
                .sidebar
                .cameras
                .get(idx)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            let fps = app.sidebar.cameras.get(idx).map(|c| c.fps).unwrap_or(0.0);
            let is_audio = app
                .audio_states
                .get(idx)
                .map(|s| s.is_audible())
                .unwrap_or(false);
            let is_selected = app.sidebar.selected == Some(idx);
            let is_hovered = app.hovered_cell == Some(idx);
            let is_recording = status == CameraStatus::Recording;
            let has_picture = matches!(status, CameraStatus::Live | CameraStatus::Recording);

            // One border language: hairline by default, a 2px accent only for a
            // meaningful state.
            let (border_col, border_w) = if is_audio {
                (Theme::color_from_hex(colors.accent_green), 2.0)
            } else if is_recording {
                (Theme::color_from_hex(colors.accent_red), 2.0)
            } else if matches!(
                status,
                CameraStatus::Reconnecting | CameraStatus::Connecting
            ) {
                (Theme::color_from_hex(colors.accent_amber), 2.0)
            } else if is_selected {
                (Theme::color_from_hex(colors.accent_blue), 2.0)
            } else if matches!(status, CameraStatus::Paused | CameraStatus::Disabled) {
                (super::style::hex_a(colors.border, 0.6), 1.0)
            } else {
                (border_color, 1.0)
            };

            let inner: Element<'_, Message> = if has_picture && idx < app.videos.len() {
                app.videos[idx].view().map(|_| Message::FrameUpdate)
            } else {
                cell_overlay::placeholder_cell(
                    colors,
                    name.clone(),
                    &status,
                    app.engine
                        .backoff_states
                        .get(idx)
                        .and_then(|b| b.status_detail()),
                )
            };
            let base = iced::widget::container(inner)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(move |_: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(video_bg)),
                    border: iced::Border {
                        color: border_col,
                        width: border_w,
                        radius: tile_radius,
                    },
                    ..iced::widget::container::Style::default()
                });

            let mut stack = iced::widget::stack![base];

            // Top-left: audio / recording badges.
            let mut tl = iced::widget::row![].spacing(3);
            if is_audio {
                let green = Theme::color_from_hex(colors.accent_green);
                tl = tl.push(small_badge(
                    "\u{266A}".into(),
                    green,
                    Theme::readable_on(green),
                ));
            }
            if is_recording {
                let e = app.engine.bridges[idx]
                    .lock()
                    .map(|b| b.recording_elapsed_secs())
                    .unwrap_or(0);
                let red = Theme::color_from_hex(colors.accent_red);
                // Com um daemon quem grava é ele e esta janela não sabe há quanto
                // tempo: um cronômetro local sempre zerado enganaria.
                let label = if app.daemon.is_daemon_mode() {
                    "REC".to_string()
                } else {
                    format!("REC {:02}:{:02}:{:02}", e / 3600, (e % 3600) / 60, e % 60)
                };
                tl = tl.push(small_badge(label, red, Theme::readable_on(red)));
            }
            stack = stack.push(super::pinned(tl, Horizontal::Left, Vertical::Top, 4.0));

            // Top-right: on-cell actions when selected or hovered.
            if is_selected || is_hovered {
                stack = stack.push(super::pinned(
                    cell_overlay::cell_actions(app, idx, is_selected),
                    Horizontal::Right,
                    Vertical::Top,
                    4.0,
                ));
            }

            // Bottom-left: compact name chip. fps only when the tile is the
            // focus of attention.
            if has_picture {
                let show_fps = if is_selected || is_hovered {
                    Some(fps)
                } else {
                    None
                };
                let pip = cell_overlay::status_pip_color(colors, &status);
                stack = stack.push(super::pinned(
                    cell_overlay::name_chip(
                        name.clone(),
                        pip,
                        show_fps,
                        is_selected,
                        app.daemon.detection_for(&name) == Some(true),
                    ),
                    Horizontal::Left,
                    Vertical::Bottom,
                    6.0,
                ));
            }

            stack = stack.width(Length::Fill).height(Length::Fill);
            row_widgets = row_widgets.push(clickable(stack, idx));
        }
        grid_rows = grid_rows.push(row_widgets.height(Length::Fill));
    }

    iced::widget::container(grid_rows)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(GAP)
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(gutter_bg)),
            ..iced::widget::container::Style::default()
        })
        .into()
}
