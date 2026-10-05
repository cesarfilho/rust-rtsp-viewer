//! The "Inspetor" tab — a curated view of the selected camera.
//!
//! Header + Stream + Rede cards up top, diagnostics chips when something is
//! wrong, and the long tail of raw counters folded into an "Avançado" block.

use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Element, Length};

use crate::domain::diagnostics::Severity;
use crate::ui::theme::{Theme, ThemeColors};

use super::types::{CameraStatus, Message, Sidebar};

fn fmt_hms(secs: u64) -> String {
    format!("{:02}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
}

/// A `label ............ value` row, value right-aligned and optionally tinted.
fn metric<'a>(colors: ThemeColors, label: &'a str, value: String, tint: Option<iced::Color>) -> Element<'a, Message> {
    let vcolor = tint.unwrap_or_else(|| Theme::color_from_hex(colors.text));
    row![
        text(label)
            .size(Theme::TEXT_BODY)
            .color(Theme::color_from_hex(colors.text_secondary)),
        iced::widget::horizontal_space(),
        text(value).size(Theme::TEXT_BODY).color(vcolor),
    ]
    .align_y(iced::Alignment::Center)
    .into()
}

fn card<'a>(theme: Theme, title: &'a str, rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let colors = theme.colors();
    let mut col = column![text(title)
        .size(Theme::TEXT_CAPTION - 1)
        .color(Theme::color_from_hex(colors.text_tertiary))]
    .spacing(Theme::SPACE_2);
    for r in rows {
        col = col.push(r);
    }
    container(col)
        .width(Length::Fill)
        .padding(Theme::SPACE_3)
        .style(style_card(theme))
        .into()
}

use crate::ui::view::style::card as style_card;

/// green if `v <= good`, amber if `v <= warn`, else red.
fn tone(colors: ThemeColors, v: f64, good: f64, warn: f64) -> iced::Color {
    if v <= good {
        Theme::color_from_hex(colors.status_live)
    } else if v <= warn {
        Theme::color_from_hex(colors.accent_amber)
    } else {
        Theme::color_from_hex(colors.accent_red)
    }
}

pub(super) fn info_view(sidebar: &Sidebar, colors: ThemeColors, theme: Theme) -> Element<'_, Message> {
    let secondary = Theme::color_from_hex(colors.text_secondary);

    let Some(idx) = sidebar.selected else {
        return placeholder("Selecione uma câmera", secondary);
    };
    let Some(cam) = sidebar.cameras.get(idx) else {
        return placeholder("Câmera não encontrada", secondary);
    };
    let Some(m) = &sidebar.selected_metrics else {
        return placeholder("Sem dados ainda…", secondary);
    };

    // ── Header ────────────────────────────────────────────────────────────
    let status_c = match cam.status {
        CameraStatus::Live => Theme::color_from_hex(colors.status_live),
        CameraStatus::Recording => Theme::color_from_hex(colors.accent_red),
        CameraStatus::Reconnecting | CameraStatus::Connecting => {
            Theme::color_from_hex(colors.accent_amber)
        }
        CameraStatus::Offline => Theme::color_from_hex(colors.accent_red),
        CameraStatus::Paused | CameraStatus::Disabled => {
            Theme::color_from_hex(colors.status_disabled)
        }
    };
    let badge = container(
        text(cam.status.label_pt())
            .size(Theme::TEXT_CAPTION - 1)
            .color(Theme::readable_on(status_c)),
    )
    .padding(iced::Padding::from([1, 6]))
    .style(move |_: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(status_c)),
        border: iced::Border { radius: 3.0.into(), ..iced::Border::default() },
        ..container::Style::default()
    });
    let header = column![
        row![
            text(&cam.name).size(Theme::TEXT_TITLE).color(Theme::color_from_hex(colors.text)),
            iced::widget::horizontal_space(),
            badge,
        ]
        .align_y(iced::Alignment::Center),
        text(format!("no ar há {}", fmt_hms(m.uptime_secs))).size(Theme::TEXT_CAPTION).color(secondary),
    ]
    .spacing(Theme::SPACE_1);

    // ── Diagnostics ──────────────────────────────────────────────────────
    let mut body = column![header].spacing(Theme::SPACE_3).padding(Theme::SPACE_3).width(Length::Fill);
    if !sidebar.diagnostics.is_empty() {
        let mut dc = column![].spacing(Theme::SPACE_1);
        for h in &sidebar.diagnostics {
            let c = match h.severity {
                Severity::Healthy => Theme::color_from_hex(colors.accent_blue),
                Severity::Degraded | Severity::Warning => Theme::color_from_hex(colors.accent_amber),
                Severity::Critical | Severity::Stalled => Theme::color_from_hex(colors.accent_red),
            };
            dc = dc.push(
                container(
                    text(format!("{} · {}", h.metric, h.cause))
                        .size(Theme::TEXT_CAPTION)
                        .color(c),
                )
                .padding(iced::Padding::from([2, 8]))
                .style(move |_: &iced::Theme| container::Style {
                    background: Some(iced::Background::Color(iced::Color { a: 0.10, ..c })),
                    border: iced::Border { color: c, width: 1.0, radius: Theme::RADIUS_SM.into() },
                    ..container::Style::default()
                }),
            );
        }
        body = body.push(dc);
    }

    // ── Stream card ──────────────────────────────────────────────────────
    let res = match (m.width, m.height) {
        (Some(w), Some(h)) => format!("{w}×{h}"),
        _ => "—".into(),
    };
    let fps_row = row![
        text("FPS").size(Theme::TEXT_BODY).color(secondary),
        iced::widget::horizontal_space(),
        super::sparkline(&m.fps_history, 48.0, 14.0, Theme::color_from_hex(colors.status_live)),
        text(format!("{:.0}", m.fps)).size(Theme::TEXT_BODY).color(Theme::color_from_hex(colors.text)),
    ]
    .spacing(Theme::SPACE_2)
    .align_y(iced::Alignment::Center);
    let mut stream_rows = vec![
        metric(colors, "Codec", m.codec.clone().unwrap_or_else(|| "—".into()), None),
        metric(colors, "Decoder", m.decoder.clone().unwrap_or_else(|| "—".into()), None),
        metric(colors, "Via", decoder_via(m.decoder.as_deref(), m.decoder_hw), None),
        metric(colors, "Resolução", res, None),
    ];
    if let Some(q) = m.stream_quality {
        stream_rows.push(metric(colors, "Fluxo", q.to_string(), None));
    }
    stream_rows.push(fps_row.into());
    stream_rows.push(metric(colors, "Bitrate", cam.bitrate.clone(), None));
    body = body.push(card(theme, "STREAM", stream_rows));

    // ── Rede card ────────────────────────────────────────────────────────
    let lat = m.latency_ms.unwrap_or(0);
    let jit = m.jitter_ms.unwrap_or(0);
    let loss = if m.packet_stats.available {
        m.packet_stats.loss_pct().unwrap_or(0.0)
    } else {
        0.0
    };
    body = body.push(card(
        theme,
        "REDE",
        vec![
            metric(
                colors,
                "Latência",
                m.latency_ms.map(|v| format!("{v} ms")).unwrap_or_else(|| "—".into()),
                Some(tone(colors, lat as f64, 200.0, 400.0)),
            ),
            metric(
                colors,
                "Jitter",
                m.jitter_ms.map(|v| format!("{v} ms")).unwrap_or_else(|| "—".into()),
                Some(tone(colors, jit as f64, 30.0, 80.0)),
            ),
            metric(
                colors,
                "Perda",
                if m.packet_stats.available { format!("{loss:.2}%") } else { "—".into() },
                Some(tone(colors, loss, 0.5, 2.0)),
            ),
            metric(colors, "Reconexões", m.reconnects.to_string(), None),
        ],
    ));

    // ── Gravação (só quando relevante) ───────────────────────────────────
    if m.is_recording {
        body = body.push(card(
            theme,
            "GRAVAÇÃO",
            vec![metric(
                colors,
                "Ativa há",
                fmt_hms(m.recording_elapsed_secs),
                Some(Theme::color_from_hex(colors.accent_red)),
            )],
        ));
    }

    // ── Avançado (expander) ──────────────────────────────────────────────
    let chevron = if sidebar.info_advanced { "\u{25BE}" } else { "\u{25B8}" };
    body = body.push(
        button(
            row![
                text(chevron).font(crate::ui::icons::FONT).size(Theme::TEXT_CAPTION).color(secondary),
                text("Avançado").size(Theme::TEXT_CAPTION).color(secondary),
            ]
            .spacing(6),
        )
        .padding(iced::Padding::from([2, 4]))
        .on_press(Message::ToggleInfoAdvanced)
        .style(move |_, _| button::Style {
            background: None,
            text_color: secondary,
            ..button::Style::default()
        }),
    );
    if sidebar.info_advanced {
        body = body.push(card(
            theme,
            "CONTADORES",
            vec![
                metric(colors, "Decode", m.decode_time_ms.map(|v| format!("{v} ms")).unwrap_or_else(|| "—".into()), None),
                metric(colors, "Frames perdidos", m.dropped.to_string(), None),
                metric(colors, "Erros de decode", m.decode_errors.to_string(), None),
                metric(colors, "Frames", m.frames.to_string(), None),
                metric(colors, "Luma", m.avg_luma.map(|v| v.to_string()).unwrap_or_else(|| "—".into()), None),
                metric(colors, "Parado há", m.static_secs.map(|v| format!("{v}s")).unwrap_or_else(|| "—".into()), None),
                metric(colors, "Último erro", m.last_error.clone().unwrap_or_else(|| "—".into()), None),
                metric(colors, "VU", format!("{}%", (m.audio_level * 100.0) as u32), None),
            ],
        ));
    }

    scrollable(body).height(Length::Fill).into()
}

fn placeholder(msg: &str, color: iced::Color) -> Element<'_, Message> {
    container(text(msg).color(color).size(Theme::TEXT_BODY))
        .padding(Theme::SPACE_4)
        .width(Length::Fill)
        .into()
}

/// Where decoding runs: `CPU` / `GPU`; a dash until the decoder is known.
fn decoder_via(decoder: Option<&str>, hardware: bool) -> String {
    match (decoder, hardware) {
        (None, _) => "—".into(),
        (Some(_), true) => "GPU".into(),
        (Some(_), false) => "CPU".into(),
    }
}

#[cfg(test)]
mod decoder_via_tests {
    use super::decoder_via;

    #[test]
    fn says_cpu_or_gpu() {
        assert_eq!(decoder_via(Some("avdec_h264"), false), "CPU");
        assert_eq!(decoder_via(Some("nvh264dec"), true), "GPU");
        assert_eq!(decoder_via(None, false), "—");
    }
}
