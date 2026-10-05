use iced::widget::{button, column, row, text};
use iced::{Element, Length};

use crate::ui::theme;
use crate::ui::view::style::{self, Intent};

use super::types::{Message, Sidebar, SidebarView};

impl Sidebar {
    pub fn view(&self, theme: theme::Theme) -> Element<'_, Message> {
        let colors = theme.colors();

        let tab = |view: SidebarView, label: &str| {
            let is_active = self.active_view == view;
            button(text(label.to_string()).size(11))
                .width(Length::Fill)
                .padding(iced::Padding::from([5, 6]))
                .style(style::pill(
                    theme,
                    if is_active {
                        Intent::Selected
                    } else {
                        Intent::Ghost
                    },
                ))
                .on_press(Message::TabClicked(view))
        };

        let tabs = column![
            row![
                tab(SidebarView::Cameras, "Câmeras"),
                tab(SidebarView::Info, "Inspetor"),
            ]
            .spacing(3),
            row![
                tab(SidebarView::Diagnostics, "Diagnóstico"),
                tab(SidebarView::Timeline, "Eventos"),
            ]
            .spacing(3),
        ]
        .spacing(3)
        .padding(iced::Padding::from([4, 2]))
        .width(Length::Fill);

        let content: Element<'_, Message> = match self.active_view {
            SidebarView::Cameras => super::cameras::cameras_view(self, colors),
            SidebarView::Info => super::info::info_view(self, colors, theme),
            SidebarView::Diagnostics => super::diagnostics::diagnostics_view(self, colors),
            SidebarView::Timeline => super::timeline::timeline_view(self, &self.timeline, colors),
        };

        column![tabs, content]
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(2)
            .into()
    }
}
