//! Objects that never move are not events: a sign or a banner the model reads as a person comes
//! back in the same box every time something *else* moves in the scene. A person walks; the box
//! of a real person does not stay put for minutes.
//!
//! A *track* is one spot of one class. A detection that overlaps a track (IoU ≥ [`SAME_SPOT_IOU`])
//! counts as a sighting, at most one per [`HIT_SPACING_MS`] (the model answers twice a second, so
//! counting every answer would make anyone who pauses for three seconds "static"). A spot seen in
//! [`STATIC_HITS`] different minutes within [`WINDOW_MS`], spread over at least [`MIN_SPAN_MS`],
//! is static, and stays static while it keeps showing up within [`STATIC_KEEP_MS`]. The first
//! sightings still raise events: only the repetitions are dropped.
//!
//! A sign is only *seen* when something else moves (the model only runs on motion), so at night it
//! may show up once every few minutes: few hits over a long window catch it, and the span rule
//! keeps a person who waits a couple of minutes at the same spot from being taken for a sign.
//! Static spots survive a restart ([`StaticFilter::static_spots`] / [`StaticFilter::with_spots`]).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::detect::Detection;

/// Two boxes of one class at the same spot. A fixed sign's boxes overlap with a median IoU of
/// 0.82 and over 0.6 nine times out of ten (measured on the Garagem camera).
pub const SAME_SPOT_IOU: f32 = 0.5;
/// Sightings counted at most once a minute.
pub const HIT_SPACING_MS: i64 = 60_000;
/// Seen in 3 different minutes...
pub const STATIC_HITS: usize = 3;
/// ...within 30 minutes...
pub const WINDOW_MS: i64 = 30 * 60_000;
/// ...the first and the last at least 10 minutes apart = static.
pub const MIN_SPAN_MS: i64 = 10 * 60_000;
/// A static spot is forgotten after a day without being seen.
pub const STATIC_KEEP_MS: i64 = 24 * 60 * 60_000;
/// Tracks kept per camera (the least recently seen is dropped).
const MAX_TRACKS: usize = 64;

#[derive(Debug, Clone)]
struct Track {
    spot: Detection,
    /// Counted sightings within the window, oldest first.
    hits: VecDeque<i64>,
    last_seen: i64,
    is_static: bool,
}

/// A spot taken for a fixed object, as kept on disk (normalised box, Unix ms).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticSpot {
    pub class: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub last_seen: i64,
}

/// The spots of one camera.
#[derive(Debug, Clone, Default)]
pub struct StaticFilter {
    tracks: Vec<Track>,
    /// A spot became static (or a static one was forgotten) since the last [`Self::take_changed`].
    changed: bool,
}

impl StaticFilter {
    /// A filter that already knows these fixed spots (read back after a restart).
    pub fn with_spots(spots: &[StaticSpot]) -> Self {
        let tracks = spots
            .iter()
            .map(|s| Track {
                spot: Detection {
                    class: s.class,
                    score: 0.0,
                    x: s.x,
                    y: s.y,
                    w: s.w,
                    h: s.h,
                },
                hits: VecDeque::new(),
                last_seen: s.last_seen,
                is_static: true,
            })
            .collect();
        Self {
            tracks,
            changed: false,
        }
    }

    /// The spots currently taken for fixed objects (what is worth saving).
    pub fn static_spots(&self) -> Vec<StaticSpot> {
        self.tracks
            .iter()
            .filter(|t| t.is_static)
            .map(|t| StaticSpot {
                class: t.spot.class,
                x: t.spot.x,
                y: t.spot.y,
                w: t.spot.w,
                h: t.spot.h,
                last_seen: t.last_seen,
            })
            .collect()
    }

    /// Whether the set of static spots changed since the last call.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Records `d` seen at `now_ms` and says whether it is a fixed object (drop it).
    pub fn is_static(&mut self, now_ms: i64, d: &Detection) -> bool {
        let before = self.tracks.iter().filter(|t| t.is_static).count();
        self.tracks.retain(|t| {
            let keep = if t.is_static {
                STATIC_KEEP_MS
            } else {
                WINDOW_MS
            };
            now_ms - t.last_seen <= keep
        });
        if self.tracks.iter().filter(|t| t.is_static).count() != before {
            self.changed = true;
        }
        let best = self
            .tracks
            .iter_mut()
            .filter(|t| t.spot.class == d.class)
            .map(|t| (t.spot.iou(d), t))
            .filter(|(iou, _)| *iou >= SAME_SPOT_IOU)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, t)| t);
        let Some(track) = best else {
            if self.tracks.len() >= MAX_TRACKS
                && let Some(oldest) = self
                    .tracks
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, t)| t.last_seen)
                    .map(|(i, _)| i)
            {
                self.tracks.swap_remove(oldest);
            }
            self.tracks.push(Track {
                spot: d.clone(),
                hits: VecDeque::from([now_ms]),
                last_seen: now_ms,
                is_static: false,
            });
            return false;
        };
        track.spot = d.clone();
        track.last_seen = now_ms;
        if track
            .hits
            .back()
            .is_none_or(|&last| now_ms - last >= HIT_SPACING_MS)
        {
            track.hits.push_back(now_ms);
        }
        while track.hits.front().is_some_and(|&t| now_ms - t > WINDOW_MS) {
            track.hits.pop_front();
        }
        let span = match (track.hits.front(), track.hits.back()) {
            (Some(first), Some(last)) => last - first,
            _ => 0,
        };
        if !track.is_static && track.hits.len() >= STATIC_HITS && span >= MIN_SPAN_MS {
            track.is_static = true;
            self.changed = true;
        }
        track.is_static
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60_000;

    fn person(x: f32) -> Detection {
        Detection {
            class: 0,
            score: 0.5,
            x,
            y: 0.72,
            w: 0.031,
            h: 0.11,
        }
    }

    #[test]
    fn a_sign_seen_now_and_then_becomes_static_and_stays_so() {
        let mut f = StaticFilter::default();
        // à noite a placa só aparece quando um carro passa: minutos 0, 6 e 12
        assert!(!f.is_static(0, &person(0.69)));
        assert!(!f.is_static(6 * MIN, &person(0.692)));
        assert!(
            f.is_static(12 * MIN, &person(0.69)),
            "3 vezes em 12 min: fixa"
        );
        assert!(f.take_changed() && !f.take_changed());
        // continua fixa enquanto aparecer, mesmo horas depois
        assert!(f.is_static(12 * MIN + 20 * 60 * MIN, &person(0.69)));
        // um dia inteiro sem aparecer: precisa provar de novo
        assert!(!f.is_static(12 * MIN + 46 * 60 * MIN, &person(0.69)));
        assert!(f.take_changed(), "esquecer também conta como mudança");
    }

    #[test]
    fn someone_waiting_a_few_minutes_at_one_spot_is_not_static() {
        let mut f = StaticFilter::default();
        // 8 minutos parado no ponto de ônibus, o modelo respondendo duas vezes por segundo
        for i in 0..960 {
            assert!(!f.is_static(i * 500, &person(0.40)), "resposta {i}");
        }
    }

    #[test]
    fn static_spots_survive_a_restart() {
        let mut f = StaticFilter::default();
        for m in [0, 6, 12] {
            f.is_static(m * MIN, &person(0.69));
        }
        let saved = f.static_spots();
        assert_eq!(saved.len(), 1);
        let mut g = StaticFilter::with_spots(&saved);
        assert!(
            g.is_static(13 * MIN, &person(0.69)),
            "já fixa depois de reiniciar"
        );
        assert!(!g.is_static(13 * MIN, &person(0.2)), "outro ponto não");
    }

    #[test]
    fn people_walking_past_the_same_sidewalk_never_build_a_track() {
        let mut f = StaticFilter::default();
        // uma pessoa por minuto, cada uma num ponto diferente da calçada
        for m in 0..10 {
            let x = 0.05 + m as f32 * 0.09;
            assert!(!f.is_static(m * MIN, &person(x)), "minuto {m}");
        }
    }

    #[test]
    fn a_different_class_at_the_same_spot_is_its_own_track() {
        let mut f = StaticFilter::default();
        for m in [0, 6, 12] {
            f.is_static(m * MIN, &person(0.69));
        }
        assert!(f.is_static(13 * MIN, &person(0.69)));
        let dog = Detection {
            class: 16,
            ..person(0.69)
        };
        assert!(!f.is_static(5 * MIN, &dog));
    }

    #[test]
    fn sightings_older_than_the_window_do_not_count() {
        let mut f = StaticFilter::default();
        // a cada 16 minutos: em 30 minutos nunca há 3 avistamentos
        for k in 0..10 {
            assert!(!f.is_static(k * 16 * MIN, &person(0.69)), "vez {k}");
        }
    }
}
