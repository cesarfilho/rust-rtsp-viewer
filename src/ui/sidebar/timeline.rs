use iced::{Element, Length};
use iced::widget::{column, container, row, scrollable, text};

use crate::ui::theme::{self, ThemeColors};

use super::types::{Message, Sidebar};

pub(super) fn timeline_view<'a>(sidebar: &'a Sidebar, timeline: &'a crate::domain::timeline::EventTimeline, colors: ThemeColors) -> Element<'a, Message> {
    let text_secondary = theme::Theme::color_from_hex(colors.text_secondary);
    let text_color = theme::Theme::color_from_hex(colors.text);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let window = 3600u64;
    let from = now.saturating_sub(window);

    let events: Vec<_> = timeline.events_in_range(from, now)
        .into_iter()
        .rev()
        .take(200)
        .collect();

    if events.is_empty() {
        return column![
            text("No events in the last hour").color(text_secondary).size(12),
        ]
        .width(Length::Fill)
        .padding(8)
        .into();
    }

    let mut event_rows = column![].width(Length::Fill);
    for event in &events {
        let color = theme::Theme::color_from_hex(event.event_type.color_hex());
        let time_str = format_timestamp(event.timestamp_secs);
        let label = event.event_type.label();
        let cam_name = sidebar.cameras.get(event.camera_idx)
            .map(|c| c.name.as_str())
            .unwrap_or("???");

        let dot = container(text(""))
            .width(6)
            .height(6)
            .style(move |_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(color)),
                border: iced::Border {
                    width: 0.0,
                    color: iced::Color::TRANSPARENT,
                    radius: 3.0.into(),
                },
                ..iced::widget::container::Style::default()
            });

        let desc = event.description.as_deref().unwrap_or("");
        let detail = format!("{}{}", cam_name, if desc.is_empty() { String::new() } else { format!(" - {}", desc) });

        let row_content = row![
            dot,
            column![
                row![text(time_str).color(text_secondary).size(10), text(label).color(color).size(11)]
                    .spacing(4)
                    .align_y(iced::Alignment::Center),
                text(detail).color(text_secondary).size(9),
            ]
            .spacing(1)
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);

        event_rows = event_rows.push(row_content);
    }

    let header = text(format!("{} events (last hour)", events.len()))
        .color(text_color)
        .size(11);

    column![header, scrollable(event_rows).height(Length::Fill)]
        .width(Length::Fill)
        .spacing(4)
        .padding(4)
        .into()
}

/// Wall-clock time-of-day for an event. Uses the local timezone so the value
/// matches the user's clock and the per-camera log files (which also use
/// `chrono::Local`). Snapshot/recording *filenames* stay UTC on purpose —
/// they need to be deterministic and sort in wall-clock order for one user.
fn format_timestamp(unix_secs: u64) -> String {
    use chrono::{Local, TimeZone, Timelike};
    match Local.timestamp_opt(unix_secs as i64, 0).single() {
        Some(dt) => format!("{:02}:{:02}:{:02}", dt.hour(), dt.minute(), dt.second()),
        None => {
            let secs_of_day = (unix_secs % 86400) as u32;
            format!(
                "{:02}:{:02}:{:02}",
                secs_of_day / 3600,
                (secs_of_day % 3600) / 60,
                secs_of_day % 60
            )
        }
    }
}
