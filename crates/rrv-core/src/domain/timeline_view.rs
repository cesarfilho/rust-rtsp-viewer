//! A matemática da linha do tempo das gravações (plano 3.4, spec `ux-historico.md`):
//! intervalo visível, zoom e deslocamento, barras dos segmentos e marcas de hora.
//! Pura: a janela só desenha o que sai daqui.

const MIN_SPAN_MS: i64 = 5 * 60_000; // 5 min
const MAX_SPAN_MS: i64 = 7 * 86_400_000; // 7 dias
/// Até quanto antes de "agora" o fim do intervalo ainda conta como seguir o vivo.
const FOLLOW_SLACK_MS: i64 = 2_000;

/// O intervalo de tempo visível (Unix ms).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub from: i64,
    pub to: i64,
}

impl Span {
    /// As últimas `hours` horas até `now`.
    pub fn last_hours(now: i64, hours: i64) -> Self {
        Self {
            from: now - hours * 3_600_000,
            to: now,
        }
    }

    pub fn len(&self) -> i64 {
        self.to - self.from
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 0
    }

    /// Posição de `t` no intervalo: 0 no início, 1 no fim (pode sair de 0..1).
    pub fn frac(&self, t: i64) -> f32 {
        (t - self.from) as f32 / self.len().max(1) as f32
    }

    /// O instante na posição `frac` (0..1).
    pub fn time_at(&self, frac: f32) -> i64 {
        self.from + (frac.clamp(0.0, 1.0) as f64 * self.len() as f64) as i64
    }

    /// Aproxima (`factor` < 1) ou afasta (`factor` > 1) mantendo parado o instante que
    /// está em `anchor` (0..1): o que está sob o ponteiro não sai do lugar.
    pub fn zoomed(&self, factor: f32, anchor: f32) -> Self {
        let new_len = ((self.len() as f64 * factor as f64) as i64).clamp(MIN_SPAN_MS, MAX_SPAN_MS);
        let pivot = self.time_at(anchor);
        let from = pivot - (new_len as f64 * anchor.clamp(0.0, 1.0) as f64) as i64;
        Self {
            from,
            to: from + new_len,
        }
    }

    /// Desloca por `frac` do tamanho do intervalo (negativo = para trás no tempo).
    pub fn panned(&self, frac: f32) -> Self {
        let d = (self.len() as f64 * frac as f64) as i64;
        Self {
            from: self.from + d,
            to: self.to + d,
        }
    }

    /// Está "seguindo o vivo": o fim do intervalo é (quase) agora.
    pub fn is_following(&self, now: i64) -> bool {
        self.to >= now - FOLLOW_SLACK_MS
    }

    /// O mesmo tamanho, terminando em `now` (para a vista acompanhar as gravações novas).
    pub fn following(&self, now: i64) -> Self {
        Self {
            from: now - self.len(),
            to: now,
        }
    }

    /// Mantém a janela de tempo sem passar de `now` (não há futuro para mostrar).
    pub fn clamped_to(&self, now: i64) -> Self {
        if self.to <= now {
            *self
        } else {
            Self {
                from: self.from - (self.to - now),
                to: now,
            }
        }
    }
}

/// Um segmento, como a linha do tempo precisa dele.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentSpan {
    pub start: i64,
    /// `None` enquanto grava: vai até `now`.
    pub end: Option<i64>,
    pub motion: bool,
    pub protected: bool,
}

/// Uma barra a desenhar, em frações (0..1) do intervalo visível.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub start: f32,
    pub end: f32,
    pub motion: bool,
    pub protected: bool,
    /// O segmento ainda está sendo gravado.
    pub live: bool,
}

/// As barras dos segmentos que tocam o intervalo, cortadas nas bordas. Segmentos
/// invisíveis (menos de meio pixel de 1000) ganham uma largura mínima para ainda se verem.
pub fn bars(segments: &[SegmentSpan], span: Span, now: i64) -> Vec<Bar> {
    const MIN_WIDTH: f32 = 0.004;
    segments
        .iter()
        .filter_map(|s| {
            let end = s.end.unwrap_or(now).max(s.start);
            if end < span.from || s.start > span.to {
                return None;
            }
            let start = span.frac(s.start).max(0.0);
            let mut stop = span.frac(end).min(1.0);
            if stop - start < MIN_WIDTH {
                stop = (start + MIN_WIDTH).min(1.0);
            }
            Some(Bar {
                start,
                end: stop,
                motion: s.motion,
                protected: s.protected,
                live: s.end.is_none(),
            })
        })
        .collect()
}

/// O índice do segmento que cobre `t`, se houver.
pub fn segment_at(segments: &[SegmentSpan], t: i64, now: i64) -> Option<usize> {
    segments
        .iter()
        .position(|s| s.start <= t && t <= s.end.unwrap_or(now))
}

/// O primeiro segmento que começa depois de `t` (para pular uma lacuna).
pub fn next_after(segments: &[SegmentSpan], t: i64) -> Option<usize> {
    segments
        .iter()
        .enumerate()
        .filter(|(_, s)| s.start > t)
        .min_by_key(|(_, s)| s.start)
        .map(|(i, _)| i)
}

/// Os passos "redondos" das marcas, em ms.
const TICK_STEPS_MS: [i64; 9] = [
    60_000,
    5 * 60_000,
    15 * 60_000,
    30 * 60_000,
    3_600_000,
    3 * 3_600_000,
    6 * 3_600_000,
    12 * 3_600_000,
    86_400_000,
];

/// Marcas de hora para o intervalo: no máximo `max_ticks`, em instantes redondos no fuso
/// de `utc_offset_secs`. Cada uma é (fração 0..1, rótulo `HH:MM`, ou `dd/mm` à meia-noite).
pub fn ticks(span: Span, utc_offset_secs: i64, max_ticks: usize) -> Vec<(f32, String)> {
    if span.is_empty() || max_ticks == 0 {
        return Vec::new();
    }
    let step = TICK_STEPS_MS
        .iter()
        .copied()
        .find(|s| span.len() / s <= max_ticks as i64)
        .unwrap_or(86_400_000);
    let off = utc_offset_secs * 1000;
    // alinha no relógio local, não no UTC
    let first = ((span.from + off) / step + 1) * step - off;
    let mut out = Vec::new();
    let mut t = first;
    while t <= span.to {
        let local = (t + off).rem_euclid(86_400_000);
        let label = if local == 0 {
            let days = (t + off).div_euclid(86_400_000);
            let (_, m, d) = civil_from_days(days);
            format!("{d:02}/{m:02}")
        } else {
            format!("{:02}:{:02}", local / 3_600_000, (local / 60_000) % 60)
        };
        out.push((span.frac(t), label));
        t += step;
    }
    out
}

/// Dias desde 1970-01-01 → (ano, mês, dia). (Algoritmo de H. Hinnant.)
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: i64 = 3_600_000;
    const NOW: i64 = 1_000 * 86_400_000 + 12 * H; // meio-dia UTC

    #[test]
    fn frac_and_time_at_are_inverse() {
        let s = Span::last_hours(NOW, 24);
        assert_eq!(s.frac(s.from), 0.0);
        assert_eq!(s.frac(s.to), 1.0);
        assert!((s.frac(s.time_at(0.25)) - 0.25).abs() < 1e-4);
        assert_eq!(s.time_at(-3.0), s.from, "fora de 0..1 fica nas bordas");
    }

    #[test]
    fn zoom_keeps_the_instant_under_the_pointer_still() {
        let s = Span::last_hours(NOW, 24);
        let anchor = 0.75;
        let pivot = s.time_at(anchor);
        let z = s.zoomed(0.5, anchor);
        assert_eq!(z.len(), 12 * H);
        assert!((z.frac(pivot) - anchor).abs() < 1e-3);
    }

    #[test]
    fn zoom_is_clamped_between_5_minutes_and_7_days() {
        let s = Span::last_hours(NOW, 1);
        assert_eq!(s.zoomed(0.0001, 0.5).len(), 5 * 60_000);
        assert_eq!(s.zoomed(10_000.0, 0.5).len(), 7 * 86_400_000);
    }

    #[test]
    fn pan_moves_by_a_fraction_of_the_span_and_never_into_the_future() {
        let s = Span::last_hours(NOW, 10);
        let back = s.panned(-0.5);
        assert_eq!((back.from, back.to), (s.from - 5 * H, s.to - 5 * H));
        let fwd = s.panned(0.5).clamped_to(NOW);
        assert_eq!(fwd.to, NOW);
        assert_eq!(fwd.len(), 10 * H, "o tamanho não muda");
    }

    #[test]
    fn following_slides_the_window_to_now_keeping_its_size() {
        let s = Span::last_hours(NOW, 6);
        assert!(s.is_following(NOW + 1_500), "folga de 2 s");
        assert!(
            !s.is_following(NOW + 60_000),
            "um minuto atrás já não segue"
        );
        let later = NOW + 90_000;
        let f = s.following(later);
        assert_eq!((f.to, f.len()), (later, 6 * H));
        // quem olha o passado não é arrastado
        assert!(!s.panned(-0.5).is_following(NOW));
    }

    #[test]
    fn bars_are_clipped_to_the_span_and_open_segments_run_to_now() {
        let s = Span {
            from: 1_000,
            to: 2_000,
        };
        let segs = [
            SegmentSpan {
                start: 0,
                end: Some(500),
                motion: false,
                protected: false,
            }, // fora
            SegmentSpan {
                start: 500,
                end: Some(1_250),
                motion: true,
                protected: false,
            }, // corta o início
            SegmentSpan {
                start: 1_800,
                end: None,
                motion: false,
                protected: true,
            },
            SegmentSpan {
                start: 2_500,
                end: Some(3_000),
                motion: false,
                protected: false,
            }, // fora
        ];
        let b = bars(&segs, s, 2_000);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].start, b[0].end, b[0].motion), (0.0, 0.25, true));
        assert!((b[1].start - 0.8).abs() < 1e-6 && (b[1].end - 1.0).abs() < 1e-6);
        assert!(b[1].live && b[1].protected);
    }

    #[test]
    fn a_tiny_segment_still_gets_a_visible_bar() {
        let s = Span::last_hours(NOW, 24);
        let b = bars(
            &[SegmentSpan {
                start: NOW - H,
                end: Some(NOW - H + 10),
                motion: false,
                protected: false,
            }],
            s,
            NOW,
        );
        assert!(b[0].end - b[0].start >= 0.004);
    }

    #[test]
    fn segment_at_and_next_after_find_the_gap_edges() {
        let segs = [
            SegmentSpan {
                start: 100,
                end: Some(200),
                motion: false,
                protected: false,
            },
            SegmentSpan {
                start: 400,
                end: Some(500),
                motion: false,
                protected: false,
            },
        ];
        assert_eq!(segment_at(&segs, 150, 1_000), Some(0));
        assert_eq!(segment_at(&segs, 300, 1_000), None, "na lacuna");
        assert_eq!(next_after(&segs, 300), Some(1));
        assert_eq!(next_after(&segs, 600), None);
    }

    #[test]
    fn ticks_are_round_local_times_and_bounded() {
        // 24 h terminando às 12:00 UTC, em UTC-3: marcas redondas no relógio local
        let s = Span::last_hours(NOW, 24);
        let t = ticks(s, -3 * 3600, 8);
        assert!(!t.is_empty() && t.len() <= 8, "{t:?}");
        assert!(t.iter().all(|(f, _)| (0.0..=1.0).contains(f)));
        assert!(
            t.iter().all(|(_, l)| l.ends_with(":00") || l.contains('/')),
            "{t:?}"
        );
        // com 12:00 UTC = 09:00 local, e passo de 3 h, aparece 06:00 local
        assert!(t.iter().any(|(_, l)| l == "06:00"), "{t:?}");
    }

    #[test]
    fn midnight_is_labelled_with_the_date() {
        // meia-noite local (UTC) do dia 1000 desde 1970 = 1972-09-27
        let s = Span {
            from: 1_000 * 86_400_000 - 6 * H,
            to: 1_000 * 86_400_000 + 6 * H,
        };
        let t = ticks(s, 0, 6);
        assert!(t.iter().any(|(_, l)| l == "27/09"), "{t:?}");
    }

    #[test]
    fn an_empty_span_has_no_ticks() {
        assert!(ticks(Span { from: 5, to: 5 }, 0, 8).is_empty());
    }
}
