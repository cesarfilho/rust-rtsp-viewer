pub mod types;
pub mod view;
pub(super) mod cameras;
pub(super) mod info;
pub(super) mod diagnostics;
pub(super) mod timeline;

pub use types::*;

use iced::widget::{container, row, text};
use iced::{Element, Length};

/// A tiny bar-chart of recent values (fps history), normalised to its own
/// recent maximum. No canvas — a row of thin rounded bars.
pub(super) fn sparkline<M: 'static>(
    values: &[f64],
    width: f32,
    height: f32,
    color: iced::Color,
) -> Element<'static, M> {
    let n = 12usize;
    let tail: Vec<f64> = values.iter().rev().take(n).rev().copied().collect();
    let max = tail.iter().cloned().fold(1.0_f64, f64::max);
    let bar_w = (width / n as f32).max(1.0);
    let mut r = row![].spacing(1.0).align_y(iced::Alignment::End);
    for i in 0..n {
        let v = tail.get(i).copied().unwrap_or(0.0);
        let h = ((v / max) as f32 * height).clamp(1.0, height);
        r = r.push(
            container(text(""))
                .width(bar_w)
                .height(h)
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(iced::Background::Color(color)),
                    border: iced::Border {
                        radius: 1.0.into(),
                        ..iced::Border::default()
                    },
                    ..container::Style::default()
                }),
        );
    }
    container(r)
        .width(Length::Fixed(width))
        .height(Length::Fixed(height))
        .into()
}
