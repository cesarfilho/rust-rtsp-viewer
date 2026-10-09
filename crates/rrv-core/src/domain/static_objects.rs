//! Objects that never move are not events: a sign or a banner the model reads as a person comes
//! back in the same box every time something *else* moves in the scene. A person walks; the box
//! of a real person does not stay put for minutes.
//!
//! A *track* is one spot of one class. A detection that overlaps a track (IoU ≥ [`SAME_SPOT_IOU`])
//! counts as a sighting, at most one per [`HIT_SPACING_MS`] (the model answers twice a second, so
//! counting every answer would make anyone who pauses for three seconds "static"). A spot seen in
//! [`STATIC_HITS`] different minutes within [`WINDOW_MS`] is static, and stays static while it
//! keeps showing up within [`STATIC_KEEP_MS`]. The first sightings still raise events: only the
//! repetitions are dropped.

use std::collections::VecDeque;

use super::detect::Detection;

/// Two boxes of one class at the same spot. A fixed sign's boxes overlap with a median IoU of
/// 0.82 and over 0.6 nine times out of ten (measured on the Garagem camera).
pub const SAME_SPOT_IOU: f32 = 0.5;
/// Sightings counted at most once a minute.
pub const HIT_SPACING_MS: i64 = 60_000;
/// Seen in 5 different minutes...
pub const STATIC_HITS: usize = 5;
/// ...within 10 minutes = static.
pub const WINDOW_MS: i64 = 10 * 60_000;
/// A static spot is forgotten after an hour without being seen (the next day it must earn it again).
pub const STATIC_KEEP_MS: i64 = 60 * 60_000;
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

/// The spots of one camera.
#[derive(Debug, Clone, Default)]
pub struct StaticFilter {
    tracks: Vec<Track>,
}

impl StaticFilter {
    /// Records `d` seen at `now_ms` and says whether it is a fixed object (drop it).
    pub fn is_static(&mut self, now_ms: i64, d: &Detection) -> bool {
        self.tracks.retain(|t| {
            let keep = if t.is_static {
                STATIC_KEEP_MS
            } else {
                WINDOW_MS
            };
            now_ms - t.last_seen <= keep
        });
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
        if track.hits.len() >= STATIC_HITS {
            track.is_static = true;
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
    fn a_sign_seen_in_five_minutes_becomes_static_and_stays_so() {
        let mut f = StaticFilter::default();
        // minutos 0..4: ainda conta como evento
        for m in 0..4 {
            assert!(!f.is_static(m * MIN, &person(0.69)), "minuto {m}");
        }
        // o quinto minuto com a caixa (quase) no mesmo lugar: fixo
        assert!(f.is_static(4 * MIN, &person(0.692)));
        // continua fixo enquanto aparecer, mesmo 40 min depois da última vez
        assert!(f.is_static(44 * MIN, &person(0.69)));
        // depois de mais de uma hora sem aparecer, precisa provar de novo
        assert!(!f.is_static(110 * MIN, &person(0.69)));
    }

    #[test]
    fn someone_standing_still_for_a_few_seconds_is_not_static() {
        let mut f = StaticFilter::default();
        // o modelo responde duas vezes por segundo: 3 min parado = 360 respostas, mas só 3 minutos
        for i in 0..360 {
            assert!(!f.is_static(i * 500, &person(0.40)), "resposta {i}");
        }
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
        for m in 0..5 {
            f.is_static(m * MIN, &person(0.69));
        }
        let dog = Detection {
            class: 16,
            ..person(0.69)
        };
        assert!(!f.is_static(5 * MIN, &dog));
    }

    #[test]
    fn sightings_older_than_the_window_do_not_count() {
        let mut f = StaticFilter::default();
        // a cada 3 minutos: em 10 minutos nunca há 5 avistamentos
        for k in 0..10 {
            assert!(!f.is_static(k * 3 * MIN, &person(0.69)), "vez {k}");
        }
    }
}
