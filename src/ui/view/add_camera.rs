//! A tela do assistente "Adicionar câmera": um cartão sobre uma cortina, como as confirmações do daemon.
//! O estado e os textos de cada etapa vêm de `ui::add_camera`; aqui só se desenha.

use crate::i18n::{t, tf};
use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Element, Length};

use super::super::add_camera::{AddCamera, AddMsg, Stage};
use super::super::app::App;
use super::super::message::Message;
use super::super::theme::Theme;
use super::style::{self, Intent};

const CARD_WIDTH: f32 = 460.0;

fn hex(s: &str) -> iced::Color {
    Theme::color_from_hex(s)
}

fn act(msg: AddMsg) -> Message {
    Message::AddCamera(msg)
}

fn btn<'a>(app: &App, label: &'a str, msg: AddMsg, intent: Intent) -> Element<'a, Message> {
    button(text(label).size(Theme::TEXT_BODY))
        .on_press(act(msg))
        .padding([6, 14])
        .style(style::pill(app.theme, intent))
        .into()
}

pub fn view<'a>(app: &'a App, wizard: &'a AddCamera) -> Element<'a, Message> {
    let theme = app.theme;
    let colors = theme.colors();
    let primary = hex(colors.text);
    let secondary = hex(colors.text_secondary);
    let scrim = iced::Color::from_rgba(0.0, 0.0, 0.0, 0.62);

    let title = text(t("Adicionar câmera"))
        .size(Theme::TEXT_TITLE)
        .color(primary);
    let body: Element<'a, Message> = match &wizard.stage {
        Stage::Scanning => column![
            text(t("Procurando câmeras ONVIF na rede…"))
                .size(Theme::TEXT_BODY)
                .color(secondary),
            row![
                iced::widget::space::horizontal(),
                btn(app, t("Fechar"), AddMsg::Close, Intent::Ghost)
            ],
        ]
        .spacing(Theme::SPACE_3)
        .into(),
        Stage::Pick { found } => {
            let mut list = column![].spacing(Theme::SPACE_1);
            if found.is_empty() {
                list = list.push(
                    text(t("Nenhuma câmera ONVIF respondeu. O ONVIF está ligado na câmera? Ela está na mesma rede?"))
                        .size(Theme::TEXT_BODY)
                        .color(secondary)
                        .width(Length::Fill),
                );
            }
            for (i, f) in found.iter().enumerate() {
                list = list.push(
                    button(text(AddCamera::describe(f)).size(Theme::TEXT_BODY))
                        .width(Length::Fill)
                        .padding([6, 10])
                        .on_press(act(AddMsg::Pick(i)))
                        .style(style::pill(theme, Intent::Ghost)),
                );
            }
            column![
                text(t("Escolha uma câmera"))
                    .size(Theme::TEXT_BODY)
                    .color(secondary),
                scrollable(list).height(Length::Fixed(180.0)),
                row![
                    btn(app, t("Procurar de novo"), AddMsg::Rescan, Intent::Ghost),
                    iced::widget::space::horizontal(),
                    btn(app, t("Fechar"), AddMsg::Close, Intent::Ghost),
                ]
                .spacing(Theme::SPACE_2),
            ]
            .spacing(Theme::SPACE_3)
            .into()
        }
        Stage::Credentials {
            found,
            user,
            password,
            error,
            ..
        } => {
            let mut c = column![
                text(AddCamera::describe(found))
                    .size(Theme::TEXT_BODY)
                    .color(primary),
                text_input(t("Usuário"), user)
                    .on_input(|u| act(AddMsg::User(u)))
                    .on_submit(act(AddMsg::Submit))
                    .padding(6),
                text_input(t("Senha"), password)
                    .on_input(|p| act(AddMsg::Password(p)))
                    .on_submit(act(AddMsg::Submit))
                    .secure(true)
                    .padding(6),
            ]
            .spacing(Theme::SPACE_2);
            if let Some(e) = error {
                c = c.push(
                    text(e.clone())
                        .size(Theme::TEXT_CAPTION)
                        .color(hex(colors.accent_red))
                        .width(Length::Fill),
                );
            }
            c.push(
                row![
                    btn(app, t("Voltar"), AddMsg::Back, Intent::Ghost),
                    iced::widget::space::horizontal(),
                    btn(
                        app,
                        t("Entrar e ler os streams"),
                        AddMsg::Submit,
                        Intent::Primary
                    ),
                ]
                .spacing(Theme::SPACE_2),
            )
            .spacing(Theme::SPACE_3)
            .into()
        }
        Stage::Inspecting { found, .. } => column![
            text(AddCamera::describe(found))
                .size(Theme::TEXT_BODY)
                .color(primary),
            text(t("Lendo os streams da câmera…"))
                .size(Theme::TEXT_BODY)
                .color(secondary),
        ]
        .spacing(Theme::SPACE_3)
        .into(),
        Stage::Done(done) => column![
            text(t(
                "Cole este trecho no seu config.toml e reinicie a janela:"
            ))
            .size(Theme::TEXT_BODY)
            .color(secondary)
            .width(Length::Fill),
            container(
                scrollable(
                    text(done.snippet.clone())
                        .size(Theme::TEXT_CAPTION)
                        .font(iced::Font::MONOSPACE)
                        .color(primary)
                )
                .height(Length::Fixed(150.0))
            )
            .padding(Theme::SPACE_2 as u16)
            .style(style::chip(theme, false)),
            text(AddCamera::keyring_note(done))
                .size(Theme::TEXT_CAPTION)
                .color(secondary)
                .width(Length::Fill),
            row![
                btn(
                    app,
                    t("Copiar trecho"),
                    AddMsg::CopySnippet,
                    Intent::Primary
                ),
                iced::widget::space::horizontal(),
                btn(app, t("Fechar"), AddMsg::Close, Intent::Ghost),
            ]
            .spacing(Theme::SPACE_2),
        ]
        .spacing(Theme::SPACE_3)
        .into(),
    };

    let card = container(column![title, body].spacing(Theme::SPACE_3))
        .padding(Theme::SPACE_4 as u16 + 8)
        .width(Length::Fixed(CARD_WIDTH))
        .style(style::popover(theme));
    let _ = tf; // (as frases com valores moram em `ui::add_camera`)
    container(card)
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
