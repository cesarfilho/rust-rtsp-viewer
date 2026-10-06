use crate::i18n::t;
use iced::widget::{column, row, rule, scrollable, text};
use iced::{Element, Length};

use crate::domain::diagnostics::Severity;
use crate::ui::theme::{self, ThemeColors};

use super::types::{Message, Sidebar};

fn make_separator(border_color: iced::Color) -> Element<'static, Message> {
    rule::horizontal(1)
        .style(move |_: &iced::Theme| rule::Style {
            color: border_color,
            snap: true,
            radius: 0.0.into(),
            fill_mode: rule::FillMode::Full,
        })
        .into()
}

pub(super) fn diagnostics_view(sidebar: &Sidebar, colors: ThemeColors) -> Element<'_, Message> {
    let border_color = theme::Theme::color_from_hex(colors.border);
    let text_primary = theme::Theme::color_from_hex(colors.text);
    let text_secondary = theme::Theme::color_from_hex(colors.text_secondary);

    if let Some(idx) = sidebar.selected {
        if let Some(cam) = sidebar.cameras.get(idx) {
            let hints = &sidebar.diagnostics;
            let mut hints_col = column![].spacing(2);
            for hint in hints {
                let sev_color = match hint.severity {
                    Severity::Healthy => theme::Theme::color_from_hex(colors.accent_blue),
                    Severity::Degraded => theme::Theme::color_from_hex(colors.accent_amber),
                    Severity::Warning => theme::Theme::color_from_hex(colors.accent_amber),
                    Severity::Critical => theme::Theme::color_from_hex(colors.accent_red),
                    Severity::Stalled => theme::Theme::color_from_hex(colors.accent_red),
                };
                hints_col = hints_col.push(
                    row![
                        text(format!("[{}]", hint.severity.glyph()))
                            .font(crate::ui::icons::FONT)
                            .color(sev_color)
                            .size(10),
                        text(format!("{}: {}", hint.metric, hint.cause))
                            .color(sev_color)
                            .size(10),
                    ]
                    .spacing(4),
                );
            }

            let diag_content = column![
                row![text(&cam.name).color(text_primary).size(13)]
                    .spacing(6)
                    .align_y(iced::Alignment::Center),
                make_separator(border_color),
                hints_col,
            ]
            .spacing(2)
            .padding(iced::Padding::from([4, 8]))
            .width(Length::Fill);
            scrollable(diag_content).height(Length::Fill).into()
        } else {
            scrollable(
                column![text(t("Câmera não encontrada")).color(text_secondary)]
                    .padding(iced::Padding::from([4, 8]))
                    .width(Length::Fill),
            )
            .height(Length::Fill)
            .into()
        }
    } else {
        scrollable(
            column![text(t("Selecione uma câmera")).color(text_secondary)]
                .padding(iced::Padding::from([4, 8]))
                .width(Length::Fill),
        )
        .height(Length::Fill)
        .into()
    }
}
