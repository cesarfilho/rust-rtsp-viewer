pub mod style;
pub mod menu;
pub mod toolbar;
pub mod cell_overlay;
pub mod grid_layout;
pub mod flex_layout;
pub mod overlays;

use iced::{Element, Length};

use super::app::{App, ViewFocus, TOOLBAR_HEIGHT};
use super::message::{LayoutMode, Message};
use super::theme::Theme;

/// Sidebar width in the normal layout — a touch more generous than the old
/// 15%/160–220, and still fully collapsible with `F2`.
fn sidebar_width(app: &App) -> f32 {
    if app.sidebar.visible && app.focus.is_normal() {
        (app.window_size.width * 0.16).clamp(200.0, 260.0)
    } else {
        0.0
    }
}

pub fn view(app: &App) -> Element<'_, Message> {
    let colors = app.theme.colors();
    let bg_color = Theme::color_from_hex(colors.background);

    let main_content: Element<'_, Message> = match app.focus {
        ViewFocus::Normal => normal_layout(app, bg_color),
        ViewFocus::Immersive => immersive_layout(app, bg_color),
        ViewFocus::Spotlight(idx) => spotlight_layout(app, idx, bg_color),
    };

    // Help is modal — it takes the whole screen. Otherwise the context menu,
    // overflow menu and toasts are independent layers that coexist.
    if app.show_help {
        return overlays::help_overlay(app, main_content);
    }

    let mut layers = iced::widget::stack![main_content];
    // A menu gets a transparent full-window backdrop so a click anywhere else
    // dismisses it.
    if let Some(menu) = toolbar::overflow_menu_layer(app) {
        layers = layers.push(dismiss_backdrop(Message::ToggleOverflowMenu));
        layers = layers.push(menu);
    }
    if let Some(menu) = overlays::context_menu_layer(app) {
        layers = layers.push(dismiss_backdrop(Message::DismissContextMenu));
        layers = layers.push(menu);
    }
    if !app.toasts.is_empty() {
        layers = layers.push(
            overlays::toast_overlay(app)
                .align_bottom(40.0)
                .align_right(16.0)
                .width(Length::Shrink),
        );
    }
    layers.into()
}

/// Full chrome: toolbar + sidebar + video area.
fn normal_layout(app: &App, bg_color: iced::Color) -> Element<'_, Message> {
    let sw = sidebar_width(app);
    let sidebar: Element<'_, Message> = if sw > 0.0 {
        track_pointer(
            iced::widget::container(app.sidebar.view(app.theme).map(Message::Sidebar))
                .width(sw)
                .height(Length::Fill)
                .style(style::sidebar_panel(app.theme))
                .into(),
            0.0,
            TOOLBAR_HEIGHT,
        )
    } else {
        iced::widget::column![].into()
    };

    let available_height = app.window_size.height - TOOLBAR_HEIGHT;
    let video_area = track_pointer(
        match app.layout_mode {
            LayoutMode::Grid => grid_layout::grid_layout(app, sw, available_height),
            LayoutMode::Flex => flex_layout::flex_layout(app),
        },
        sw,
        TOOLBAR_HEIGHT,
    );

    let content = iced::widget::column![
        toolbar::toolbar(app),
        iced::widget::row![sidebar, video_area].height(Length::Fill),
    ]
    .width(Length::Fill)
    .height(Length::Fill);

    fill_bg(content, bg_color)
}

/// No chrome: the grid (or flex) edge-to-edge, with a reveal rail on demand.
fn immersive_layout(app: &App, bg_color: iced::Color) -> Element<'_, Message> {
    let video_area = track_pointer(
        match app.layout_mode {
            LayoutMode::Grid => grid_layout::grid_layout(app, 0.0, app.window_size.height),
            LayoutMode::Flex => flex_layout::flex_layout(app),
        },
        0.0,
        0.0,
    );
    with_reveal_rail(app, fill_bg(video_area, bg_color))
}

/// A transparent, full-window click target that emits `msg` — used to dismiss
/// an open menu when the user clicks outside it.
fn dismiss_backdrop<'a>(msg: Message) -> Element<'a, Message> {
    iced::widget::mouse_area(
        iced::widget::container(iced::widget::text(""))
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .on_press(msg)
    .into()
}

/// Wrap the video area so pointer moves over it update `App.pointer_pos` (in
/// window coordinates — `mouse_area` reports a bounds-local point, so the
/// area's own offset is added back).
fn track_pointer<'a>(el: Element<'a, Message>, ox: f32, oy: f32) -> Element<'a, Message> {
    iced::widget::mouse_area(el)
        .on_move(move |p| Message::PointerMoved(iced::Point::new(p.x + ox, p.y + oy)))
        .into()
}

/// One camera full-bleed.
fn spotlight_layout(app: &App, idx: usize, bg_color: iced::Color) -> Element<'_, Message> {
    let cell = flex_layout::spotlight_view(app, idx);
    with_reveal_rail(app, fill_bg(cell, bg_color))
}

fn with_reveal_rail<'a>(app: &'a App, base: Element<'a, Message>) -> Element<'a, Message> {
    if !app.chrome_revealed {
        return base;
    }
    iced::widget::stack![
        base,
        iced::widget::container(toolbar::chrome_rail(app))
            .width(Length::Fill)
            .align_top(0)
    ]
    .into()
}

fn fill_bg<'a>(
    content: impl Into<Element<'a, Message>>,
    bg_color: iced::Color,
) -> Element<'a, Message> {
    iced::widget::container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(move |_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(bg_color)),
            ..iced::widget::container::Style::default()
        })
        .into()
}
