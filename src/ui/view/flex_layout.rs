use crate::i18n::{t, tf};
use iced::alignment::{Horizontal, Vertical};
use iced::{Element, Length};

use super::super::app::App;
use super::super::message::Message;
use super::super::sidebar::CameraStatus;
use super::super::theme::Theme;
use super::cell_overlay;

/// One camera, full-bleed, for `ViewFocus::Spotlight`. Live video (or the
/// shared placeholder when it isn't) plus a slim bottom overlay: name · state
/// on the left, `‹ › ✕` on the right.
pub fn spotlight_view(app: &App, idx: usize) -> Element<'_, Message> {
    let colors = app.theme.colors();
    let video_bg = iced::Color::from_rgb(0.0, 0.0, 0.0);

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
    let is_live = matches!(status, CameraStatus::Live | CameraStatus::Recording);

    let picture: Element<'_, Message> = if is_live && idx < app.videos.len() {
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
    let picture = iced::widget::container(picture)
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(video_bg)),
            ..iced::widget::container::Style::default()
        });

    let fps = app.sidebar.cameras.get(idx).map(|c| c.fps).unwrap_or(0.0);
    let right = format!("{:.0} fps", fps);
    let bar = iced::widget::container(
        iced::widget::row![
            iced::widget::text(name)
                .size(Theme::TEXT_EMPHASIS)
                .color(iced::Color::from_rgb(0.95, 0.95, 0.97)),
            iced::widget::text(status.label_pt())
                .size(Theme::TEXT_CAPTION)
                .color(iced::Color::from_rgb(0.7, 0.7, 0.75)),
            iced::widget::text(right)
                .size(Theme::TEXT_CAPTION)
                .color(iced::Color::from_rgb(0.7, 0.7, 0.75)),
            iced::widget::space::horizontal(),
            cell_overlay::feature_actions(app, idx),
            iced::widget::space::horizontal().width(Theme::SPACE_3),
            nav_btn("\u{2039}", Message::SpotlightStep(false)),
            nav_btn("\u{203A}", Message::SpotlightStep(true)),
            nav_btn("\u{2715}", Message::ExitFocus),
        ]
        .spacing(Theme::SPACE_3)
        .align_y(iced::Alignment::Center),
    )
    .width(Length::Fill)
    .padding(iced::Padding::from([6, 12]))
    .style(|_: &iced::Theme| super::style::caption_scrim());

    let mut layers = iced::widget::stack![picture]
        .width(Length::Fill)
        .height(Length::Fill);
    if is_live && let Some(boxes) = detections_for(app, idx) {
        let (_, w, h, _) = app.engine.bridges[idx]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .read_frame();
        layers = layers.push(crate::ui::detections_overlay::layer(
            boxes, w as f32, h as f32,
        ));
    }
    if let Some(edit) = app.zone_edit.as_ref().filter(|e| e.camera_idx == idx) {
        layers = layers.push(zone_editor_layer(app, idx, edit));
        layers = layers.push(super::pinned(
            iced::widget::mouse_area(zone_editor_bar(edit)).on_press(Message::Noop),
            Horizontal::Center,
            Vertical::Top,
            0.0,
        ));
    }
    layers
        .push(super::pinned(
            iced::widget::mouse_area(bar).on_press(Message::Noop),
            Horizontal::Left,
            Vertical::Bottom,
            0.0,
        ))
        .into()
}

/// The objects the daemon sees now on camera `idx` (matched by name, never by index), or `None`.
fn detections_for(app: &App, idx: usize) -> Option<Vec<crate::ipc::protocol::WireBox>> {
    let name = &app.sidebar.cameras.get(idx)?.name;
    let cam = app.daemon.cameras.iter().find(|c| &c.name == name)?;
    (!cam.detections.is_empty()).then(|| cam.detections.clone())
}

/// Canvas over the picture that captures clicks while zones are being drawn.
fn zone_editor_layer<'a>(
    app: &'a App,
    idx: usize,
    edit: &'a super::super::app::ZoneEdit,
) -> Element<'a, Message> {
    let (_, w, h, _) = app.engine.bridges[idx]
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .read_frame();
    let program = crate::ui::zone_editor::ZoneEditorProgram {
        zones: app
            .engine
            .zones
            .get(idx)
            .map(|c| c.zones.clone())
            .unwrap_or_default(),
        adding_mode: true,
        temp_vertices: edit.temp_vertices.clone(),
        video_width: w as f32,
        video_height: h as f32,
    };
    crate::ui::zone_editor::zone_editor_widget_element(program).map(|m| match m {
        crate::ui::zone_editor::ZoneEditorMessage::VertexAdded(x, y) => Message::ZoneVertex(x, y),
        crate::ui::zone_editor::ZoneEditorMessage::CloseRequested => Message::ZoneFinish,
    })
}

/// Hint + actions for the zone editor, pinned to the top of the picture.
fn zone_editor_bar(edit: &super::super::app::ZoneEdit) -> Element<'_, Message> {
    let hint = if edit.temp_vertices.is_empty() {
        t("Clique no vídeo para marcar os cantos da zona").to_string()
    } else {
        tf(
            "{} ponto(s) · Enter ou clique no 1º ponto conclui",
            &[&edit.temp_vertices.len()],
        )
    };
    iced::widget::container(
        iced::widget::row![
            iced::widget::text(hint)
                .size(Theme::TEXT_CAPTION)
                .color(iced::Color::from_rgb(0.95, 0.95, 0.97)),
            nav_btn(t("Concluir"), Message::ZoneFinish),
            nav_btn(t("Desfazer"), Message::ZoneUndo),
            nav_btn(t("Limpar"), Message::ZoneClear),
            nav_btn(t("Sair"), Message::ZoneCancel),
        ]
        .spacing(Theme::SPACE_3)
        .align_y(iced::Alignment::Center),
    )
    .padding(iced::Padding::from([6, 12]))
    .style(|_: &iced::Theme| super::style::caption_scrim())
    .into()
}

fn nav_btn(glyph: &str, msg: Message) -> Element<'_, Message> {
    iced::widget::button(
        iced::widget::text(glyph.to_string())
            .font(crate::ui::icons::FONT)
            .size(15)
            .color(iced::Color::from_rgb(0.92, 0.92, 0.94)),
    )
    .padding(iced::Padding::from([2, 8]))
    .on_press(msg)
    .style(|_, status| iced::widget::button::Style {
        background: Some(iced::Background::Color(match status {
            iced::widget::button::Status::Hovered | iced::widget::button::Status::Pressed => {
                iced::Color::from_rgba(1.0, 1.0, 1.0, 0.15)
            }
            _ => iced::Color::TRANSPARENT,
        })),
        text_color: iced::Color::from_rgb(0.92, 0.92, 0.94),
        border: iced::Border {
            radius: Theme::RADIUS_SM.into(),
            ..iced::Border::default()
        },
        ..iced::widget::button::Style::default()
    })
    .into()
}

pub fn flex_layout(app: &App) -> Element<'_, Message> {
    if app.videos.is_empty() {
        return iced::widget::container(iced::widget::text(t("Nenhuma câmera configurada")))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    }

    let colors = app.theme.colors();
    let border_color = Theme::color_from_hex(colors.border);
    let video_bg = iced::Color::from_rgb(0.0, 0.0, 0.0);
    let btn_active_bg = Theme::color_from_hex(colors.accent_blue);
    let tile_radius: iced::border::Radius = Theme::RADIUS_MD.into();

    let main_idx = app.flex_main_idx.min(app.videos.len().saturating_sub(1));
    let is_main_disabled = !app
        .engine
        .camera_enabled
        .get(main_idx)
        .copied()
        .unwrap_or(true);
    let is_audio_main = app
        .audio_states
        .get(main_idx)
        .map(|s| s.is_audible())
        .unwrap_or(false);
    let is_recording_main = app
        .sidebar
        .cameras
        .get(main_idx)
        .map(|c| c.status == super::super::sidebar::CameraStatus::Recording)
        .unwrap_or(false);
    let main_border_color = if is_main_disabled {
        iced::Color::from_rgba(0.4, 0.4, 0.4, 0.3)
    } else if is_audio_main {
        Theme::color_from_hex(colors.accent_green)
    } else if is_recording_main {
        Theme::color_from_hex(colors.accent_red)
    } else {
        border_color
    };
    let main_border_width = if is_main_disabled || is_audio_main || is_recording_main {
        2.0
    } else {
        1.0
    };

    let main_cell: iced::widget::Container<'_, Message> = if is_main_disabled {
        let cam_name = app
            .sidebar
            .cameras
            .get(main_idx)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        iced::widget::container(
            iced::widget::column![
                iced::widget::text("\u{26A0}")
                    .font(crate::ui::icons::FONT)
                    .color(iced::Color::from_rgba(0.5, 0.5, 0.5, 0.6))
                    .size(32),
                iced::widget::text(cam_name)
                    .color(Theme::color_from_hex(colors.text))
                    .size(14),
                iced::widget::text(t("DESATIVADA"))
                    .color(iced::Color::from_rgba(0.5, 0.5, 0.5, 0.6))
                    .size(11),
            ]
            .spacing(4)
            .align_x(iced::Alignment::Center),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(video_bg)),
            border: iced::Border {
                color: main_border_color,
                width: main_border_width,
                radius: tile_radius,
            },
            ..iced::widget::container::Style::default()
        })
    } else {
        iced::widget::container(app.videos[main_idx].view().map(|_| Message::FrameUpdate))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(video_bg)),
                border: iced::Border {
                    color: main_border_color,
                    width: main_border_width,
                    radius: tile_radius,
                },
                ..iced::widget::container::Style::default()
            })
    };

    let main: Element<'_, Message> = if is_audio_main {
        let badge = iced::widget::container(
            iced::widget::text("\u{266A} AUD")
                .font(crate::ui::icons::FONT)
                .color(iced::Color::from_rgb(0.0, 0.0, 0.0))
                .size(10),
        )
        .padding(2)
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(Theme::color_from_hex(
                colors.accent_green,
            ))),
            border: iced::Border {
                radius: 3.0.into(),
                ..iced::Border::default()
            },
            ..iced::widget::container::Style::default()
        });

        iced::widget::stack![
            main_cell,
            super::pinned(badge, Horizontal::Left, Vertical::Top, 4.0)
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        main_cell.into()
    };

    let mut thumb_col = iced::widget::column![].spacing(4);
    for i in 0..app.videos.len() {
        if !app.engine.camera_enabled.get(i).copied().unwrap_or(true) {
            continue;
        }
        let is_selected = i == main_idx;
        let is_audio_thumb = app
            .audio_states
            .get(i)
            .map(|s| s.is_audible())
            .unwrap_or(false);
        let thumb_border_color = if is_audio_thumb {
            Theme::color_from_hex(colors.accent_green)
        } else if is_selected {
            btn_active_bg
        } else {
            border_color
        };
        let thumb_border_width = if is_audio_thumb || is_selected {
            2.0
        } else {
            1.0
        };

        // Only the main camera streams in flex; the others show one grabbed frame.
        // The main camera's own thumbnail stays a label: one video id drawn twice in a
        // frame shares one GPU rect, so the live picture would land only in the thumbnail.
        let has_frame = app.engine.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .has_frame();
        let thumb_pic: Element<'_, Message> = if !is_selected
            && (has_frame || app.engine.active_stream.get(i).copied().unwrap_or(false))
        {
            app.videos[i]
                .view()
                .map(move |_| Message::FlexMainSelected(i))
        } else {
            let name = app
                .sidebar
                .cameras
                .get(i)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            iced::widget::container(
                iced::widget::text(name)
                    .size(Theme::TEXT_CAPTION)
                    .color(Theme::color_from_hex(colors.text_secondary)),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
        };
        let thumb_cell = iced::widget::container(thumb_pic)
            .width(160.0)
            .height(90.0)
            .style(move |_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(video_bg)),
                border: iced::Border {
                    color: thumb_border_color,
                    width: thumb_border_width,
                    radius: (Theme::RADIUS_SM).into(),
                },
                ..iced::widget::container::Style::default()
            });

        let thumb_inner: Element<'_, Message> = if is_audio_thumb {
            let badge = iced::widget::container(
                iced::widget::text("\u{266A}")
                    .font(crate::ui::icons::FONT)
                    .color(iced::Color::from_rgb(0.0, 0.0, 0.0))
                    .size(8),
            )
            .padding(1)
            .style(move |_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(Theme::color_from_hex(
                    colors.accent_green,
                ))),
                border: iced::Border {
                    radius: 2.0.into(),
                    ..iced::Border::default()
                },
                ..iced::widget::container::Style::default()
            });

            iced::widget::stack![
                thumb_cell,
                super::pinned(badge, Horizontal::Left, Vertical::Top, 2.0)
            ]
            .width(160.0)
            .height(90.0)
            .into()
        } else {
            thumb_cell.into()
        };

        let thumb = iced::widget::button(thumb_inner)
            .on_press(Message::FlexMainSelected(i))
            .padding(0);
        thumb_col = thumb_col.push(thumb);
    }

    // Scrolls when there are more thumbnails than fit the window height.
    let thumb_scroll = iced::widget::scrollable(thumb_col.padding(iced::Padding::ZERO.right(8.0)))
        .width(Length::Fill)
        .height(Length::Fill);

    let thumbs = iced::widget::container(thumb_scroll)
        .width(180.0)
        .height(Length::Fill)
        .padding(6)
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(Theme::color_from_hex(
                colors.surface,
            ))),
            border: iced::Border {
                color: border_color,
                width: 1.0,
                ..iced::Border::default()
            },
            ..iced::widget::container::Style::default()
        });

    iced::widget::row![main, thumbs]
        .spacing(6)
        .height(Length::Fill)
        .into()
}
