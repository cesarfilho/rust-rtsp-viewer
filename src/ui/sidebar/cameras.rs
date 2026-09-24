use iced::{Element, Length};
use iced::widget::{button, column, container, row, scrollable, text, text_input};

use crate::ui::theme::{self, ThemeColors};

use super::types::{CameraInfo, CameraStatus, Message, Sidebar};

/// Stable id for the search field so the `/` shortcut can focus it.
pub fn search_input_id() -> text_input::Id {
    text_input::Id::new("sidebar-search")
}

pub(super) fn cameras_view(sidebar: &Sidebar, colors: ThemeColors) -> Element<'_, Message> {
    let surface_color = theme::Theme::color_from_hex(colors.surface);
    let border_color = theme::Theme::color_from_hex(colors.border);
    let text_color = theme::Theme::color_from_hex(colors.text);
    let text_secondary = theme::Theme::color_from_hex(colors.text_secondary);
    let placeholder_color = theme::Theme::color_from_hex(colors.text_tertiary);
    let accent = theme::Theme::color_from_hex(colors.accent_blue);
    let active_surface = theme::Theme::color_from_hex(colors.surface_hover);

    let search = text_input("Filtrar câmeras  (/)", &sidebar.search_query)
        .id(search_input_id())
        .on_input(Message::SearchChanged)
        .on_submit(Message::SearchSubmitted)
        .padding(iced::Padding::from([2, 4]))
        .width(Length::Fill)
        .style(move |_, status| iced::widget::text_input::Style {
            background: iced::Background::Color(surface_color),
            border: iced::Border {
                color: match status {
                    iced::widget::text_input::Status::Focused => accent,
                    _ => border_color,
                },
                width: match status {
                    iced::widget::text_input::Status::Focused => 1.0,
                    _ => 0.5,
                },
                radius: theme::Theme::RADIUS_SM.into(),
            },
            icon: placeholder_color,
            placeholder: placeholder_color,
            value: text_color,
            selection: iced::Color::from_rgba(0.4, 0.6, 1.0, 0.3),
        });

    let in_group = sidebar.visible_camera_indices(sidebar.cameras.len());
    let query = sidebar.search_query.to_lowercase();
    // Rows follow the user's saved display order (the ▲/▼ buttons), then the
    // group filter and the search box.
    let ordered = crate::domain::view::apply_order(
        &sidebar.order,
        &(0..sidebar.cameras.len()).collect::<Vec<_>>(),
    );
    let filtered: Vec<(usize, &CameraInfo)> = ordered
        .iter()
        .filter_map(|&idx| sidebar.cameras.get(idx).map(|c| (idx, c)))
        .filter(|(idx, _)| in_group.contains(idx))
        .filter(|(_, cam)| {
            query.is_empty()
                || cam.name.to_lowercase().contains(&query)
                || cam.kind.to_lowercase().contains(&query)
        })
        .collect();
    let last_pos = filtered.len().saturating_sub(1);
    // The reorder arrows act on the full order, not the filtered slice, so
    // they are only meaningful (and only shown) when nothing is filtering.
    let reorderable = query.is_empty() && sidebar.active_group.is_none();

    let group_chips = group_chip_row(sidebar, colors);

    let status_color = |s: &CameraStatus| match s {
        CameraStatus::Live => theme::Theme::color_from_hex(colors.status_live),
        CameraStatus::Recording => theme::Theme::color_from_hex(colors.accent_red),
        CameraStatus::Offline => theme::Theme::color_from_hex(colors.status_offline),
        CameraStatus::Reconnecting | CameraStatus::Connecting => {
            theme::Theme::color_from_hex(colors.status_reconnecting)
        }
        CameraStatus::Disabled | CameraStatus::Paused => {
            theme::Theme::color_from_hex(colors.status_disabled)
        }
    };

    let icon_btn = move |glyph: &'static str, msg: Option<Message>| {
        let on = msg.is_some();
        let fg = if on { text_secondary } else { placeholder_color };
        let mut b = button(text(glyph).size(10).color(fg))
            .padding(iced::Padding::from([1, 4]))
            .style(move |_, s| button::Style {
                background: match s {
                    button::Status::Hovered if on => Some(iced::Background::Color(active_surface)),
                    _ => None,
                },
                text_color: fg,
                border: iced::Border::default().color(iced::Color::TRANSPARENT).width(0),
                ..button::Style::default()
            });
        if let Some(m) = msg {
            b = b.on_press(m);
        }
        b
    };

    let mut camera_rows = column![].width(Length::Fill).spacing(1);
    for (row_pos, &(idx, cam)) in filtered.iter().enumerate() {
        let is_selected = sidebar.selected == Some(idx);
        let is_hovered = sidebar.hover_row == Some(idx);
        let healthy = matches!(cam.status, CameraStatus::Live | CameraStatus::Recording);
        let name_color = if is_selected { accent } else { text_color };

        // pip + name; a second line appears only when something is wrong.
        let mut lines = column![
            row![
                container(text(""))
                    .width(6)
                    .height(6)
                    .style({
                        let c = if is_selected { accent } else { status_color(&cam.status) };
                        move |_: &iced::Theme| container::Style {
                            background: Some(iced::Background::Color(c)),
                            border: iced::Border { radius: 3.0.into(), ..iced::Border::default() },
                            ..container::Style::default()
                        }
                    }),
                text(&cam.name).color(name_color).size(theme::Theme::TEXT_BODY),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(1);
        if !healthy {
            lines = lines.push(
                text(cam.status.label_pt())
                    .color(theme::Theme::color_from_hex(colors.accent_amber))
                    .size(theme::Theme::TEXT_CAPTION - 1),
            );
        }

        let row_btn = button(lines)
            .width(Length::Fill)
            .padding(iced::Padding::from([5, 8]))
            .style(move |_, status| button::Style {
                background: match status {
                    button::Status::Hovered | button::Status::Pressed => {
                        Some(iced::Background::Color(active_surface))
                    }
                    _ if is_selected => {
                        Some(iced::Background::Color(iced::Color { a: 0.12, ..accent }))
                    }
                    _ => None,
                },
                text_color,
                border: iced::Border { radius: theme::Theme::RADIUS_SM.into(), ..iced::Border::default() },
                ..button::Style::default()
            })
            .on_press(Message::CameraClicked(idx));

        let mut r = row![row_btn].spacing(2).align_y(iced::Alignment::Center);

        if is_selected || is_hovered {
            if reorderable {
                r = r.push(column![
                    icon_btn("\u{25B2}", (row_pos > 0).then_some(Message::CameraMovedUp(idx))),
                    icon_btn("\u{25BC}", (row_pos < last_pos).then_some(Message::CameraMovedDown(idx))),
                ]);
            }
            r = r.push(icon_btn("\u{22EF}", Some(Message::ShowRowMenu(idx))));
        } else if healthy && !cam.fps_history.is_empty() {
            r = r.push(super::sparkline(
                &cam.fps_history,
                28.0,
                14.0,
                theme::Theme::color_from_hex(colors.status_live),
            ));
        }

        camera_rows = camera_rows.push(
            container(
                iced::widget::mouse_area(r)
                    .on_enter(Message::HoverRow(Some(idx)))
                    .on_exit(Message::HoverRow(None)),
            )
            .height(Length::Fixed(if healthy { 34.0 } else { 42.0 })),
        );
    }
    let camera_scroll = scrollable(camera_rows).height(Length::Fill);

    let mut col = column![search].width(Length::Fill).spacing(2);
    if let Some(chips) = group_chips {
        col = col.push(chips);
    }
    col.push(camera_scroll).into()
}

/// A wrapped row of filter chips — "Todas N" plus one per configured group
/// with its camera count. `None` when no groups are configured.
fn group_chip_row<'a>(sidebar: &'a Sidebar, colors: ThemeColors) -> Option<Element<'a, Message>> {
    if sidebar.groups.is_empty() {
        return None;
    }
    let accent = theme::Theme::color_from_hex(colors.accent_blue);
    let text_secondary = theme::Theme::color_from_hex(colors.text_secondary);

    let chip = |label: String, selected: bool, target: Option<usize>| -> Element<'a, Message> {
        let fg = if selected { accent } else { text_secondary };
        button(text(label).size(theme::Theme::TEXT_CAPTION - 1).color(fg))
            .padding(iced::Padding::from([2, 7]))
            .on_press(Message::GroupSelected(target))
            .style(move |_, status| button::Style {
                background: Some(iced::Background::Color(if selected {
                    iced::Color { a: 0.16, ..accent }
                } else if matches!(status, button::Status::Hovered) {
                    iced::Color { a: 0.06, ..accent }
                } else {
                    iced::Color::TRANSPARENT
                })),
                text_color: fg,
                border: iced::Border {
                    color: if selected { accent } else { iced::Color::TRANSPARENT },
                    width: if selected { 1.0 } else { 0.0 },
                    radius: theme::Theme::RADIUS_SM.into(),
                },
                ..button::Style::default()
            })
            .into()
    };

    let mut items: Vec<Element<'a, Message>> = vec![chip(
        format!("Todas {}", sidebar.cameras.len()),
        sidebar.active_group.is_none(),
        None,
    )];
    for (i, g) in sidebar.groups.iter().enumerate() {
        items.push(chip(
            format!("{} {}", g.name, g.camera_indices.len()),
            sidebar.active_group == Some(i),
            Some(i),
        ));
    }
    Some(iced::widget::row(items).spacing(3).wrap().into())
}
