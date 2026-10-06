//! A parte visual da conexão com o daemon (spec `docs/specs/ux-daemon.md`): o
//! chip da barra, o menu do chip, o banner de aviso e as confirmações. O estado
//! e os textos vêm de `ui::daemon`; aqui só se desenha.

use crate::i18n::t;
use iced::widget::{button, column, container, row, text};
use iced::{Element, Length};

use super::super::app::App;
use super::super::daemon::{Modal, Mode, Tone};
use super::super::message::Message;
use super::super::theme::Theme;
use super::menu::{MENU_WIDTH, menu_row};
use super::style::{self, Intent};

/// Altura do banner fino sob a barra.
pub const BANNER_HEIGHT: f32 = 32.0;

fn hex(s: &str) -> iced::Color {
    Theme::color_from_hex(s)
}

/// A cor do tom. Nunca é a única pista: o chip também muda de forma e de rótulo.
fn tone_color(theme: Theme, tone: Tone) -> iced::Color {
    let c = theme.colors();
    hex(match tone {
        Tone::Good => c.status_live,
        Tone::Warning => c.accent_amber,
        Tone::Error => c.accent_red,
        Tone::Neutral => c.text_tertiary,
    })
}

/// O chip do daemon, à esquerda do medidor de saúde. Em janela estreita só a
/// forma da bolinha (o rótulo está no menu).
pub fn chip(app: &App, compact: bool) -> Element<'_, Message> {
    let d = &app.daemon;
    let theme = app.theme;
    let glyph = text(d.glyph())
        .font(super::super::icons::FONT)
        .size(Theme::TEXT_BODY)
        .color(tone_color(theme, d.tone()));
    let mut content = row![glyph]
        .spacing(Theme::SPACE_1)
        .align_y(iced::Alignment::Center);
    if !compact {
        content = content.push(
            text(d.chip_label())
                .size(Theme::TEXT_CAPTION)
                .color(hex(theme.colors().text_secondary)),
        );
    }
    button(content)
        .on_press(Message::ToggleDaemonMenu)
        .padding([4, 8])
        .style(style::pill(
            theme,
            if d.menu_open {
                Intent::Selected
            } else {
                Intent::Ghost
            },
        ))
        .into()
}

/// O menu do chip: estado em texto corrido e as ações que fazem sentido agora.
pub fn menu(app: &App) -> Element<'_, Message> {
    let d = &app.daemon;
    let theme = app.theme;
    let colors = theme.colors();
    let trouble = matches!(d.mode, Mode::Lost | Mode::Incompatible | Mode::NoPermission);

    let mut col = column![
        container(
            text(d.summary())
                .size(Theme::TEXT_BODY)
                .color(hex(colors.text))
        )
        .padding(iced::Padding::from([8, 10]))
    ]
    .spacing(2);
    if let Some(detail) = d.detail.as_ref().filter(|_| trouble) {
        col = col.push(
            container(
                text(detail.clone())
                    .size(Theme::TEXT_CAPTION)
                    .color(hex(colors.text_tertiary)),
            )
            .padding(iced::Padding::from([0, 10])),
        );
    }
    col = col.push(container(iced::widget::horizontal_rule(1)).padding([4, 0]));
    if trouble {
        col = col.push(menu_row(
            theme,
            "\u{21BB}",
            t("Reconectar agora"),
            None,
            Message::DaemonReconnect,
            false,
        ));
        col = col.push(menu_row(
            theme,
            "\u{25A3}",
            t("Usar o motor local…"),
            None,
            Message::AskUseLocalEngine,
            true,
        ));
    }
    col = col.push(menu_row(
        theme,
        "\u{25E7}",
        t("Copiar comando para iniciar o daemon"),
        None,
        Message::CopyDaemonStartCommand,
        false,
    ));
    col = col.push(menu_row(
        theme,
        "\u{25E7}",
        t("Copiar caminho do socket"),
        None,
        Message::CopyDaemonSocket,
        false,
    ));
    container(col.padding(4))
        .width(MENU_WIDTH + 60.0)
        .style(style::popover(theme))
        .into()
}

/// O aviso fino sob a barra (não cobre o vídeo): só existe quando há algo a dizer.
pub fn banner(app: &App) -> Option<Element<'_, Message>> {
    let b = app.daemon.banner()?;
    let theme = app.theme;
    let colors = theme.colors();
    let accent = tone_color(theme, b.tone);
    // Sobre o tom translúcido do banner o texto secundário ficava em 4,4:1 no
    // tema Dark; os botões usam o texto principal (ver o teste de contraste).
    let ink = hex(colors.text);
    let small = |label: &'static str, msg: Message, intent: Intent| {
        button(text(label).size(Theme::TEXT_CAPTION))
            .on_press(msg)
            .padding([2, 8])
            .style(move |t: &iced::Theme, status| {
                let mut st = style::pill(theme, intent)(t, status);
                st.text_color = ink;
                st
            })
    };
    let bar = row![
        text(app.daemon.glyph())
            .font(super::super::icons::FONT)
            .size(Theme::TEXT_BODY)
            .color(accent),
        text(b.text)
            .size(Theme::TEXT_CAPTION)
            .color(hex(colors.text))
            .width(Length::Fill),
        small(
            t("Reconectar agora"),
            Message::DaemonReconnect,
            Intent::Ghost
        ),
        small(
            t("Usar motor local"),
            Message::AskUseLocalEngine,
            Intent::Ghost
        ),
    ]
    .spacing(Theme::SPACE_2)
    .align_y(iced::Alignment::Center);
    Some(
        container(bar)
            .padding([0, Theme::SPACE_3 as u16])
            .width(Length::Fill)
            .height(BANNER_HEIGHT)
            .align_y(iced::Alignment::Center)
            .style(move |_: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(iced::Color { a: 0.16, ..accent })),
                border: iced::Border {
                    color: accent,
                    width: 1.0,
                    radius: 0.0.into(),
                },
                ..container::Style::default()
            })
            .into(),
    )
}

/// Uma confirmação: o fundo escurece, o teclado fica preso nela (Enter aceita,
/// Esc recusa) e a ação destrutiva é a que tem o botão em destaque.
pub fn modal<'a>(app: &'a App, modal: Modal) -> Element<'a, Message> {
    let theme = app.theme;
    let colors = theme.colors();
    let scrim = iced::Color::from_rgba(0.0, 0.0, 0.0, 0.62);
    let card = column![
        text(modal.title())
            .size(Theme::TEXT_TITLE)
            .color(hex(colors.text)),
        text(modal.body())
            .size(Theme::TEXT_BODY)
            .color(hex(colors.text_secondary))
            .width(360),
        row![
            iced::widget::horizontal_space(),
            button(text(t("Cancelar")).size(Theme::TEXT_BODY))
                .on_press(Message::ModalCancel)
                .padding([6, 14])
                .style(style::pill(theme, Intent::Ghost)),
            button(text(modal.confirm_label()).size(Theme::TEXT_BODY))
                .on_press(Message::ModalConfirm)
                .padding([6, 14])
                .style(style::pill(theme, Intent::Danger)),
        ]
        .spacing(Theme::SPACE_2),
    ]
    .spacing(Theme::SPACE_3)
    .padding(Theme::SPACE_4 as u16 + 8);
    container(container(card).style(style::popover(theme)))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(move |_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(scrim)),
            ..container::Style::default()
        })
        .into()
}
