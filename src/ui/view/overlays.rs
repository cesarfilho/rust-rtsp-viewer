use iced::{Element, Length};

use super::super::app::App;
use super::super::message::Message;
use super::super::theme::Theme;
use super::style;

pub fn toast_overlay(app: &App) -> iced::widget::Container<'_, Message> {
    let colors = app.theme.colors();
    let toast_accent = Theme::color_from_hex(colors.accent_green);
    let toast_bg = Theme::color_from_hex(colors.surface_elevated);
    let toast_text_color = Theme::color_from_hex(colors.text);

    let mut toast_col = iced::widget::column![].spacing(6);
    for toast in &app.toasts {
        let label = iced::widget::text(&toast.message)
            .color(toast_text_color)
            .size(12);
        toast_col = toast_col.push(
            iced::widget::container(label)
                .padding(iced::Padding::from([8, 14]))
                .style(move |_: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(toast_bg)),
                    border: iced::Border {
                        color: toast_accent,
                        width: 1.0,
                        radius: Theme::RADIUS_MD.into(),
                    },
                    shadow: iced::Shadow {
                        color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.4),
                        offset: iced::Vector::new(0.0, 6.0),
                        blur_radius: 24.0,
                    },
                    ..iced::widget::container::Style::default()
                }),
        );
    }
    iced::widget::container(toast_col)
        .width(Length::Shrink)
        .padding(8)
}

pub fn help_overlay<'a>(app: &App, main_content: Element<'a, Message>) -> Element<'a, Message> {
    let theme = app.theme;
    let colors = theme.colors();
    let scrim = iced::Color::from_rgba(0.0, 0.0, 0.0, 0.62);
    let primary = Theme::color_from_hex(colors.text);
    let secondary = Theme::color_from_hex(colors.text_secondary);
    let accent = Theme::color_from_hex(colors.accent_blue);

    let row = |keys: &'a str, desc: &'a str| {
        iced::widget::row![
            iced::widget::container(iced::widget::text(keys).color(accent).size(12))
                .width(120),
            iced::widget::text(desc).color(primary).size(12),
        ]
        .spacing(8)
    };

    let help_content = iced::widget::column![
        iced::widget::text("Atalhos de teclado").color(primary).size(16),
        iced::widget::horizontal_rule(1),
        row("h", "Modo imersivo (esconde toda a interface)"),
        row("f / Enter", "Spotlight da câmera selecionada"),
        row("\u{2190} / \u{2192}", "Câmera anterior / próxima (no spotlight)"),
        row("F11", "Tela cheia do SO + imersivo"),
        row("Tab", "Alternar layout (grade / flex)"),
        row("g", "Densidade da grade (Auto/2x2/3x3/4x4)"),
        row("[ / ]  ·  PgUp / PgDn", "Página anterior / seguinte"),
        row("c", "Ligar / desligar o carrossel de páginas"),
        row("1 - 9", "Selecionar câmera"),
        row("Space / k", "Alternar seleção"),
        row("s / F12", "Snapshot"),
        row("r", "Iniciar / parar gravação"),
        row("m  ·  + / -", "Mudo  ·  volume"),
        row("F2", "Mostrar / ocultar a sidebar"),
        row("F3", "Alternar aba da sidebar"),
        row("/", "Filtrar câmeras"),
        row("Esc", "Voltar: spotlight \u{2192} imersivo \u{2192} menu \u{2192} busca"),
        row("?", "Mostrar / ocultar esta ajuda"),
        row("Ctrl+Q", "Sair"),
        iced::widget::horizontal_rule(1),
        iced::widget::text("Clique para selecionar \u{00B7} duplo-clique para spotlight \u{00B7} clique direito para ações")
            .color(secondary)
            .size(11),
    ]
    .spacing(6)
    .padding(24)
    .width(420);

    let help_popup = iced::widget::container(help_content).style(style::popover(theme));

    iced::widget::stack![
        main_content,
        iced::widget::container(help_popup)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(move |_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(scrim)),
                ..iced::widget::container::Style::default()
            })
    ]
    .into()
}

/// The right-click command menu as a standalone layer, anchored at the pointer
/// (clamped so it stays on screen). Same surface as the toolbar `⋯` menu.
pub fn context_menu_layer(app: &App) -> Option<Element<'_, Message>> {
    let ctx = app.context_menu.as_ref()?;

    // Rough menu extent for edge-clamping. Height varies with the camera
    // section; ~420 covers the tallest case.
    let menu_w = super::menu::MENU_WIDTH + 8.0;
    let menu_h = 420.0_f32;
    let x = app
        .pointer_pos
        .x
        .min((app.window_size.width - menu_w).max(0.0))
        .max(0.0);
    let y = app
        .pointer_pos
        .y
        .min((app.window_size.height - menu_h).max(0.0))
        .max(0.0);

    Some(
        iced::widget::container(super::menu::command_menu(app, Some(ctx.camera_idx)))
            .align_top(y)
            .align_left(x)
            .into(),
    )
}
