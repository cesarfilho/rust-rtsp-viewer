pub mod cell_overlay;
pub mod daemon;
pub mod flex_layout;
pub mod grid_layout;
pub mod menu;
pub mod overlays;
pub mod style;
pub mod toolbar;

use iced::alignment::{Horizontal, Vertical};
use iced::{Element, Length};

use super::app::{App, TOOLBAR_HEIGHT, ViewFocus};
use super::message::{LayoutMode, Message};
use super::theme::Theme;
use daemon::BANNER_HEIGHT;

/// Sidebar width in the normal layout — a touch more generous than the old
/// 15%/160–220, and still fully collapsible with `F2`.
fn sidebar_width(app: &App) -> f32 {
    if app.sidebar.visible && app.focus.is_normal() {
        (app.window_size.width * 0.16).clamp(200.0, 260.0)
    } else {
        0.0
    }
}

/// Where the video area starts: the toolbar plus, when there is one, the
/// daemon banner. Grid sizing and pointer tracking both depend on it.
fn chrome_top(app: &App) -> f32 {
    if app.focus.is_normal() && app.daemon.banner().is_some() {
        TOOLBAR_HEIGHT + BANNER_HEIGHT
    } else {
        TOOLBAR_HEIGHT
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
    if let Some(menu) = toolbar::daemon_menu_layer(app) {
        layers = layers.push(dismiss_backdrop(Message::ToggleDaemonMenu));
        layers = layers.push(menu);
    }
    if let Some(menu) = overlays::context_menu_layer(app) {
        layers = layers.push(dismiss_backdrop(Message::DismissContextMenu));
        layers = layers.push(menu);
    }
    if let Some(m) = app.modal {
        layers = layers.push(daemon::modal(app, m));
    }
    if !app.toasts.is_empty() {
        layers = layers.push(pinned(
            overlays::toast_overlay(app),
            Horizontal::Right,
            Vertical::Bottom,
            iced::Padding {
                top: 0.0,
                right: 8.0,
                bottom: 32.0,
                left: 0.0,
            },
        ));
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
            chrome_top(app),
        )
    } else {
        iced::widget::column![].into()
    };

    let available_height = app.window_size.height - chrome_top(app);
    let video_area = track_pointer(
        match app.layout_mode {
            LayoutMode::Grid => grid_layout::grid_layout(app, sw, available_height),
            LayoutMode::Flex => flex_layout::flex_layout(app),
        },
        sw,
        chrome_top(app),
    );

    let mut content = iced::widget::column![toolbar::toolbar(app)]
        .width(Length::Fill)
        .height(Length::Fill);
    if let Some(banner) = daemon::banner(app) {
        content = content.push(banner);
    }
    let content = content.push(iced::widget::row![sidebar, video_area].height(Length::Fill));

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

/// Pin `el` against an edge/corner of the layer it is stacked on, `pad` away.
///
/// Do not use `Container::align_top(x)` / `align_left(x)` for this: in iced 0.13
/// their argument is the container's *height/width*, not a margin. Used as
/// "x pixels from the edge" they shrink the container to `x` pixels and the
/// content spills out of it (a menu that resizes with the pointer, badges that
/// land off-screen). A layer-sized container with padding is the correct form;
/// it does not capture events, so layers underneath still receive them.
pub(super) fn pinned<'a>(
    el: impl Into<Element<'a, Message>>,
    h: iced::alignment::Horizontal,
    v: iced::alignment::Vertical,
    pad: impl Into<iced::Padding>,
) -> Element<'a, Message> {
    iced::widget::container(el)
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(h)
        .align_y(v)
        .padding(pad)
        .into()
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
        pinned(
            toolbar::chrome_rail(app),
            Horizontal::Left,
            Vertical::Top,
            0.0,
        )
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
