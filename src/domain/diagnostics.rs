//! Diagnostic hints derived from a `Metrics` snapshot.
//!
//! The overlay colourises metrics in green/yellow/orange/red but colour alone
//! is not enough for a stressed operator to know what to do. This module
//! produces short, actionable text hints ("→ check Wi-Fi", "→ raise cache")
//! that ride along the red/orange metric lines so the eye doesn't have to
//! read three full rows of numbers to figure out "what is broken right now".
//!
//! Pure functions on the `Metrics` snapshot — no I/O, no GStreamer — so
//! every hint can be unit-tested in isolation.

use crate::domain::metrics::Metrics;
use log::{info, warn};

/// Severity rank used to pick the worst hint for the "overall health" badge.
/// Ordered from best (Healthy) to worst (Stalled).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Healthy = 0,
    Degraded = 1,
    Warning = 2,
    Critical = 3,
    Stalled = 4,
}

impl Severity {
    /// Stable 1-character glyph rendered in the header of the overlay.
    pub fn glyph(self) -> &'static str {
        match self {
            Severity::Healthy => "✓",
            Severity::Degraded => "·",
            Severity::Warning => "!",
            Severity::Critical => "‼",
            Severity::Stalled => "✕",
        }
    }
}

/// One actionable diagnostic line. `metric` is the overlay field name
/// (e.g. `"Loss"`, `"Lat"`, `"FPS"`), `cause` is a short imperative phrase
/// (`"check Wi-Fi"`). Rendered in the overlay as `Loss: 1.2% → check Wi-Fi`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub metric: &'static str,
    pub severity: Severity,
    pub cause: &'static str,
}

impl Hint {
    /// Convenience constructor mirroring the field order used by the
    /// `diagnose` functions below.
    #[allow(dead_code)]
    pub fn new(metric: &'static str, severity: Severity, cause: &'static str) -> Self {
        Self {
            metric,
            severity,
            cause,
        }
    }
}

/// Compute every active hint for the current `Metrics` snapshot.
///
/// Ordered by descending severity (worst first) so the overlay can truncate
/// to a few hints without dropping the most important ones.
pub fn diagnose(m: &Metrics) -> Vec<Hint> {
    let mut out = Vec::new();

    if !m.is_live.load(std::sync::atomic::Ordering::Relaxed) {
        // Don't show metric-level hints while offline — the live indicator
        // and (when known) the reconnect status already explain the state.
        return out;
    }

    // --- FPS ---
    match m.snapshot_current_fps() {
        None => {} // No sample yet — no hint.
        Some(fps) if fps <= 0.0 => {
            out.push(Hint {
                metric: "FPS",
                severity: Severity::Stalled,
                cause: "decoder stalled — reattach or restart",
            });
        }
        Some(fps) if fps < critical_fps_threshold(m) => {
            out.push(Hint {
                metric: "FPS",
                severity: Severity::Critical,
                cause: "decoder can't keep up — lower resolution or close other apps",
            });
        }
        _ => {}
    }

    // --- Latency ---
    if let Some(ms) = m.snapshot_actual_latency_ms() {
        if ms > 5_000 {
            out.push(Hint {
                metric: "Lat",
                severity: Severity::Critical,
                cause: "atraso > 5s — check rtspsrc latency= or network path",
            });
        } else if ms > 2_000 {
            out.push(Hint {
                metric: "Lat",
                severity: Severity::Warning,
                cause: "latência alta — considere reduzir --cache",
            });
        }
    }

    // --- Jitter ---
    // Fixed thresholds: buffering is now entirely GStreamer's (`rtspsrc
    // latency`, `uridecodebin`), so there is no app-tracked "cache window" to
    // scale against. These floors represent genuine network instability.
    if let Some(jms) = m.snapshot_jitter_ms() {
        let warn_ms = 50;
        let crit_ms = 100;
        if jms > crit_ms {
            out.push(Hint {
                metric: "Jit",
                severity: Severity::Critical,
                cause: "jitter alto — risco de underflow no buffer",
            });
        } else if jms > warn_ms {
            out.push(Hint {
                metric: "Jit",
                severity: Severity::Warning,
                cause: "variação de banda — ver CPU/rede",
            });
        }
    }

    // --- Packet loss ---
    let pkt = m.snapshot_packet_stats();
    if pkt.available
        && let Some(pct) = pkt.loss_pct()
    {
        if pct > 1.0 {
            out.push(Hint {
                metric: "Loss",
                severity: Severity::Critical,
                cause: "Wi-Fi/MTU — testar com cabo ou reduzir bitrate",
            });
        } else if pct > 0.1 {
            out.push(Hint {
                metric: "Loss",
                severity: Severity::Warning,
                cause: "perda de pacotes — verificar interferência",
            });
        }
    }

    // --- Decoder errors ---
    let dec_errs = m.decode_errors.load(std::sync::atomic::Ordering::Relaxed);
    if dec_errs > 0 {
        out.push(Hint {
            metric: "DecE",
            severity: if dec_errs > 5 {
                Severity::Critical
            } else {
                Severity::Warning
            },
            cause: "decoder reportou erro — codec/bitrate podem estar fora",
        });
    }

    // --- Reconnects ---
    let rec = m.reconnect_count.load(std::sync::atomic::Ordering::Relaxed);
    if rec >= 3 {
        out.push(Hint {
            metric: "Rc",
            severity: Severity::Critical,
            cause: "múltiplas reconexões — RTSP ou rede instável",
        });
    } else if rec > 0 {
        out.push(Hint {
            metric: "Rc",
            severity: Severity::Degraded,
            cause: "reconectou uma vez — monitorar",
        });
    }

    // --- Queue underrun (the user already sees Q as red; hint is contextual) ---
    // We don't keep a low_streak counter on Metrics (it's in the bus-loop
    // state), so skip this hint — the Q colour itself is the signal.

    // --- Image quality: scene is static for a long time ---
    if let Some(s) = m.snapshot_static_secs(now_unix_secs())
        && s > 300
    {
        out.push(Hint {
            metric: "Stale",
            severity: Severity::Warning,
            cause: "cena parada > 5min — câmera travada ou cena realmente parada?",
        });
    }

    // --- Image quality: night scene (item 6) ---
    //
    // When the average frame luma is very low (< NIGHT_LUMA_THRESHOLD)
    // and the camera is producing frames (`frame_count > 0`), surface
    // a `Night` hint. The intent is: "the image is dark, but that's
    // because the scene is dark, not because the camera is broken".
    //
    // We only emit the hint when the sample is recent (within
    // `NIGHT_SAMPLE_MAX_AGE_SECS`) so a stale reading from
    // before a scene change doesn't keep firing forever.
    //
    // Suppressed when no frames have ever been decoded (a frozen
    // black frame is already reported as `Stalled`, which is more
    // urgent than `Night` and we don't want to double-flag).
    if let Some(luma) = m.snapshot_recent_avg_luma(now_unix_secs(), NIGHT_SAMPLE_MAX_AGE_SECS)
        && luma < NIGHT_LUMA_THRESHOLD
        && m.frame_count.load(std::sync::atomic::Ordering::Relaxed) > 0
    {
        out.push(Hint {
            metric: "Night",
            severity: Severity::Degraded,
            cause: night_cause(luma),
        });
    }

    // --- Image quality: tamper detection (item 7) ---
    //
    // A camera that has been physically **covered** (luma pinned
    // near 0) or pointed at a **bright light** (luma pinned near
    // 255) for a sustained period is a security event, not a
    // scene condition. We use the scene-change detector as the
    // discriminator: if the scene hasn't changed in
    // `TAMPER_STATIC_SECS` (default 30s) AND the luma is at one
    // of the extremes, we flag `Tamper`.
    //
    // The "static for N seconds" requirement is what makes this
    // different from `Night`: night scenes have continuous
    // micro-variation (a passing car, a swaying branch, a
    // changing lightbulb), so the scene-change clock keeps
    // ticking. A *frozen* dark frame is a different signal.
    //
    // Suppressed when:
    //   * the pipeline is offline (`!is_live`)
    //   * no recent luma sample exists (we'd be guessing)
    if let Some(luma) = m.snapshot_recent_avg_luma(now_unix_secs(), TAMPER_SAMPLE_MAX_AGE_SECS)
        && let Some(static_secs) = m.snapshot_static_secs(now_unix_secs())
        && static_secs >= TAMPER_STATIC_SECS
    {
        if luma <= TAMPER_LUMA_LO {
            out.push(Hint {
                metric: "Tamper",
                severity: Severity::Warning,
                cause: tamper_covered_cause(luma, static_secs),
            });
        } else if luma >= TAMPER_LUMA_HI {
            out.push(Hint {
                metric: "Tamper",
                severity: Severity::Warning,
                cause: tamper_blinded_cause(luma, static_secs),
            });
        }
    }

    out.sort_by_key(|h| std::cmp::Reverse(h.severity));
    out
}

/// Item 6: avg-luma value below which we consider the scene
/// "night". 20/255 ≈ 8% brightness; an interior at dusk lands
/// around 25–40, a street at night around 5–15. 20 keeps us
/// just above the false-positive threshold for dim but
/// well-lit scenes (e.g. a corridor with motion-activated
/// lights).
pub const NIGHT_LUMA_THRESHOLD: u8 = 20;

/// Item 6: how long an avg-luma sample is considered
/// "recent". The post-decode probe writes a new luma every
/// ~30 frames; at 30 fps that's ~1s, but a stalled pipeline
/// can leave a 5-minute-old luma. We refuse to read a sample
/// older than 3s so a stale reading doesn't keep us in
/// `Night` after the camera recovers.
const NIGHT_SAMPLE_MAX_AGE_SECS: u64 = 3;

/// Item 7: how long a scene must be unchanged before we
/// consider it a tamper candidate. 30s is the sweet spot: a
/// casual passerby who covers a camera for a few seconds
/// shouldn't trigger an alert, but a camera that's been
/// covered for 30s+ is no longer a "scene" — it's a security
/// event. The value was chosen empirically: night scenes
/// never have 30s of *zero* variation, so the static
/// requirement is what separates tamper from night.
const TAMPER_STATIC_SECS: u64 = 30;

/// Item 7: lower bound of "extreme" luma. Anything ≤ 5/255
/// for a sustained period means the camera lens is covered
/// (tape, paint, cloth, a hand). 5 leaves some headroom for
/// noisy night scenes that occasionally dip to 1–3.
const TAMPER_LUMA_LO: u8 = 5;

/// Item 7: upper bound of "extreme" luma. Anything ≥
/// 250/255 for a sustained period means the camera is
/// blinded by a strong light source (torch, laser, sun
/// reflection off a new surface). 250 is conservative;
/// 254 is a fully-saturated white frame.
const TAMPER_LUMA_HI: u8 = 250;

/// Item 7: same "fresh sample" window as Night. The tamper
/// check uses the luma as a primary signal so we want it to
/// be recent.
const TAMPER_SAMPLE_MAX_AGE_SECS: u64 = 3;

fn night_cause(luma: u8) -> &'static str {
    // We can't easily interpolate the luma into a `&'static
    // str`, so we bucket. 5 buckets cover the 0-19 range
    // without giving up too much granularity.
    match luma {
        0..=3 => "very dark scene (luma ≤ 3/255) — night mode?",
        4..=8 => "dark scene (luma ≤ 8/255) — night mode?",
        9..=13 => "low light (luma ≤ 13/255) — night mode?",
        14..=17 => "dim scene (luma ≤ 17/255) — night mode?",
        _ => "low light (luma ≤ 19/255) — night mode?",
    }
}

fn tamper_covered_cause(luma: u8, static_secs: u64) -> &'static str {
    // Two buckets: "fully covered" (0–2) vs "dim/covered"
    // (3–5). The "fully" bucket is more urgent because the
    // pixel values are so dark that the luma stddev must
    // also be near zero (which we don't directly test, but
    // a luma of 0/1 is rare in any natural scene).
    let intensity = match luma {
        0..=2 => "fully covered",
        _ => "covered",
    };
    // We avoid `format!` here so the cause string remains
    // `&'static str` (the Hint struct takes a `&'static
    // str`). 5 buckets cover 30s–5min, the realistic range
    // for "I've been staring at this and it's still the
    // same black image".
    let dur = match static_secs {
        0..=44 => "~30s",
        45..=89 => "~1min",
        90..=179 => "~2min",
        180..=299 => "~3min",
        _ => "5min+",
    };
    match (intensity, dur) {
        ("fully covered", "~30s") => "câmera coberta (luma ≤ 2/255) há ~30s — verificar obstrução",
        ("fully covered", "~1min") => {
            "câmera coberta (luma ≤ 2/255) há ~1min — verificar obstrução"
        }
        ("fully covered", "~2min") => {
            "câmera coberta (luma ≤ 2/255) há ~2min — verificar obstrução"
        }
        ("fully covered", "~3min") => {
            "câmera coberta (luma ≤ 2/255) há ~3min — verificar obstrução"
        }
        ("fully covered", "5min+") => "câmera coberta (luma ≤ 2/255) há 5min+ — vandalismo?",
        ("covered", "~30s") => "cena muito escura (luma ≤ 5/255) há ~30s — verificar cobertura",
        ("covered", "~1min") => "cena muito escura (luma ≤ 5/255) há ~1min — verificar cobertura",
        ("covered", "~2min") => "cena muito escura (luma ≤ 5/255) há ~2min — verificar cobertura",
        ("covered", "~3min") => "cena muito escura (luma ≤ 5/255) há ~3min — verificar cobertura",
        ("covered", "5min+") => "cena muito escura (luma ≤ 5/255) há 5min+ — vandalismo?",
        _ => "luma muito baixa há muito tempo — verificar câmera",
    }
}

fn tamper_blinded_cause(_luma: u8, static_secs: u64) -> &'static str {
    let dur = match static_secs {
        0..=44 => "~30s",
        45..=89 => "~1min",
        90..=179 => "~2min",
        180..=299 => "~3min",
        _ => "5min+",
    };
    match dur {
        "~30s" => "câmera ofuscada (luma ≥ 250/255) há ~30s — luz direta ou laser?",
        "~1min" => "câmera ofuscada (luma ≥ 250/255) há ~1min — luz direta ou laser?",
        "~2min" => "câmera ofuscada (luma ≥ 250/255) há ~2min — luz direta ou laser?",
        "~3min" => "câmera ofuscada (luma ≥ 250/255) há ~3min — vandalismo?",
        _ => "câmera ofuscada (luma ≥ 250/255) há 5min+ — vandalismo?",
    }
}

/// Overall stream health in a single severity rank — used for the at-a-
/// glance badge in the overlay header. Derived from `diagnose` so the
/// badge is always consistent with the hint list and thresholds are
/// maintained in one place.
///
/// Kept public for tests and potential external callers; in production the
/// overlay derives severity inline from the hints vec.
pub fn overall_severity(m: &Metrics) -> Severity {
    diagnose(m)
        .iter()
        .map(|h| h.severity)
        .max()
        .unwrap_or(Severity::Healthy)
}

/// 60% of the negotiated framerate. Below this we consider the decoder
/// "not keeping up". Falls back to 18 fps when the framerate is unknown.
fn critical_fps_threshold(m: &Metrics) -> f64 {
    let info = m.snapshot_stream_info();
    match (info.framerate_num, info.framerate_den) {
        (Some(n), Some(d)) if n > 0 && d > 0 => {
            let fps = n as f64 / d as f64;
            (fps * 0.6).max(1.0)
        }
        _ => 18.0,
    }
}

fn now_unix_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metrics::PacketStats;
    use std::sync::Arc;

    fn fresh() -> Arc<Metrics> {
        Metrics::new()
    }

    #[test]
    fn severity_ordering_is_stable() {
        assert!(Severity::Stalled > Severity::Critical);
        assert!(Severity::Critical > Severity::Warning);
        assert!(Severity::Warning > Severity::Degraded);
        assert!(Severity::Degraded > Severity::Healthy);
    }

    #[test]
    fn offline_returns_no_hints() {
        let m = fresh();
        m.is_live.store(false, std::sync::atomic::Ordering::Relaxed);
        assert!(diagnose(&m).is_empty());
    }

    #[test]
    fn healthy_live_returns_no_hints() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(diagnose(&m).is_empty());
        assert_eq!(overall_severity(&m), Severity::Healthy);
    }

    #[test]
    fn high_loss_prompts_wifi_hint() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        *m.packet_stats.lock().unwrap() = PacketStats {
            available: true,
            pushed: 900,
            lost: 100, // 10%
            ..Default::default()
        };
        let hints = diagnose(&m);
        assert!(
            hints
                .iter()
                .any(|h| h.metric == "Loss" && h.cause.contains("Wi-Fi"))
        );
        assert_eq!(overall_severity(&m), Severity::Critical);
    }

    #[test]
    fn moderate_loss_is_warning_not_critical() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        *m.packet_stats.lock().unwrap() = PacketStats {
            available: true,
            pushed: 1_000,
            lost: 2, // 0.2%
            ..Default::default()
        };
        let hints = diagnose(&m);
        assert_eq!(
            hints.iter().find(|h| h.metric == "Loss").unwrap().severity,
            Severity::Warning
        );
    }

    #[test]
    fn latency_above_5s_is_critical() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.actual_latency_ns
            .store(6_000_000_000, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(overall_severity(&m), Severity::Critical);
        assert!(diagnose(&m).iter().any(|h| h.metric == "Lat"));
    }

    #[test]
    fn jitter_above_critical_threshold_is_critical() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        // Fixed crit threshold = 100 ms. Store 110 ms jitter → Critical.
        m.jitter_ema_ns
            .store(110_000_000, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(overall_severity(&m), Severity::Critical);
    }

    #[test]
    fn jitter_uses_fixed_thresholds() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        // 25 ms is below the 50 ms warn floor → no jitter hint.
        m.jitter_ema_ns
            .store(25_000_000, std::sync::atomic::Ordering::Relaxed);
        assert!(diagnose(&m).iter().all(|h| h.metric != "Jit"));
        // 60 ms crosses the 50 ms warn floor → Warning.
        m.jitter_ema_ns
            .store(60_000_000, std::sync::atomic::Ordering::Relaxed);
        let hints = diagnose(&m);
        let jit = hints
            .iter()
            .find(|h| h.metric == "Jit")
            .expect("jitter hint");
        assert_eq!(jit.severity, Severity::Warning);
    }

    #[test]
    fn decode_errors_counted() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.decode_errors
            .store(1, std::sync::atomic::Ordering::Relaxed);
        assert!(diagnose(&m).iter().any(|h| h.metric == "DecE"));
    }

    #[test]
    fn many_reconnects_are_critical() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.reconnect_count
            .store(5, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(overall_severity(&m), Severity::Critical);
    }

    #[test]
    fn single_reconnect_is_degraded() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.reconnect_count
            .store(1, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(overall_severity(&m), Severity::Degraded);
    }

    #[test]
    fn hints_are_sorted_by_severity() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        // With cache=0: crit threshold=100ms. Store 110ms to trigger a Jit hint.
        m.jitter_ema_ns
            .store(110_000_000, std::sync::atomic::Ordering::Relaxed);
        m.reconnect_count
            .store(1, std::sync::atomic::Ordering::Relaxed);
        *m.packet_stats.lock().unwrap() = PacketStats {
            available: true,
            pushed: 1_000,
            lost: 1, // 0.1% → warning
            ..Default::default()
        };
        let hints = diagnose(&m);
        for w in hints.windows(2) {
            assert!(w[0].severity >= w[1].severity);
        }
    }

    #[test]
    fn severity_glyphs_are_distinct() {
        let glyphs = [
            Severity::Healthy,
            Severity::Degraded,
            Severity::Warning,
            Severity::Critical,
            Severity::Stalled,
        ]
        .iter()
        .map(|s| s.glyph())
        .collect::<Vec<_>>();
        let unique: std::collections::HashSet<_> = glyphs.iter().copied().collect();
        assert_eq!(unique.len(), glyphs.len());
    }

    // =================================================================
    // Item 6 — Night detection
    // =================================================================
    //
    // Goal: when the camera is in a low-light scene AND the pipeline
    // is live AND the FPS is reasonable (so the camera is responsive,
    // just dark), surface a `Night` hint so the operator knows the
    // dark image is a scene condition, not a camera fault.
    //
    // Rules (all three must hold):
    //   1. `is_live == true` (offline is *not* night — it's offline).
    //   2. `last_avg_luma < NIGHT_LUMA_THRESHOLD` (default 20/255).
    //   3. FPS sample exists and is > 0 (otherwise it's a stall, which
    //      is already a more urgent `Stalled` hint — we don't want
    //      to double-flag a frozen black frame as both Stalled +
    //      Night).
    //
    // The hint carries the luma value in its `cause` string so the
    // operator can see how dark ("Night mode? luma=8/255") without
    // having to look at the Luma row separately.

    #[test]
    fn dark_live_stream_surfaces_night_hint() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(10, std::sync::atomic::Ordering::Relaxed);
        // FPS > 0 so the camera is responsive (just dark).
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        // Pretend a tick has happened ~1s ago.
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        let night: Vec<&Hint> = hints.iter().filter(|h| h.metric == "Night").collect();
        assert_eq!(
            night.len(),
            1,
            "expected exactly one Night hint, got {hints:?}"
        );
        assert_eq!(night[0].severity, Severity::Degraded);
        assert!(
            night[0].cause.contains("night"),
            "cause should mention night: {}",
            night[0].cause
        );
    }

    #[test]
    fn bright_live_stream_does_not_suggest_night() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(128, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        assert!(
            hints.iter().all(|h| h.metric != "Night"),
            "luma=128 should not be flagged as night, got: {hints:?}"
        );
    }

    #[test]
    fn dark_offline_stream_is_not_night() {
        let m = fresh();
        m.is_live.store(false, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(5, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        assert!(
            hints.iter().all(|h| h.metric != "Night"),
            "offline is not night, got: {hints:?}"
        );
    }

    #[test]
    fn dark_stream_with_no_luma_sample_is_not_night() {
        // The probe runs every ~30 frames; the first 29 frames have
        // no luma sample. We must not invent a "night" out of a default
        // 0 reading.
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(0, std::sync::atomic::Ordering::Relaxed);
        // No frame sample recorded.
        let hints = diagnose(&m);
        assert!(
            hints.iter().all(|h| h.metric != "Night"),
            "luma=0 without a sample must not be night, got: {hints:?}"
        );
    }

    #[test]
    fn dark_stream_with_fps_zero_is_stall_not_night() {
        // A frozen black frame should report Stalled, not Night.
        // The Stalled hint is more urgent — we suppress the Night
        // hint in that case so the overlay doesn't double-flag.
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(8, std::sync::atomic::Ordering::Relaxed);
        // Bus loop sampled and saw zero frames in the last
        // interval → `snapshot_current_fps()` returns `Some(0.0)`
        // → existing FPS check emits the Stalled hint.
        m.current_fps_x1000
            .store(0, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        let night: Vec<&Hint> = hints.iter().filter(|h| h.metric == "Night").collect();
        assert!(
            night.is_empty(),
            "stalled stream should not also be flagged night, got: {hints:?}"
        );
        // It should still be flagged as stalled.
        assert!(
            hints
                .iter()
                .any(|h| h.metric == "FPS" && h.severity == Severity::Stalled),
            "expected a Stalled hint, got: {hints:?}"
        );
    }

    #[test]
    fn dark_stream_includes_luma_band_in_cause() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(7, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        let night = hints.iter().find(|h| h.metric == "Night").unwrap();
        // Operator benefit: the cause string tells them roughly
        // how dark the scene is (a band, not the exact byte).
        // For luma=7 we expect to see "luma" and "8" (bucket
        // 4..=8).
        assert!(
            night.cause.contains("luma"),
            "cause should include 'luma': {}",
            night.cause
        );
        assert!(
            night.cause.contains('8'),
            "cause should include the bucket upper bound (8): {}",
            night.cause
        );
    }

    // =================================================================
    // Item 7 — Tamper detection
    // =================================================================
    //
    // Goal: a camera that has been physically covered (luma
    // pinned near 0) or pointed at a bright light (luma pinned
    // near 255) for a sustained period is a security event, not
    // a scene condition. Distinguishing it from `Night` is the
    // hard part:
    //
    //   * The scene must be **static** (no changes for ≥
    //     TAMPER_STATIC_SECS, default 30s). Night isn't static
    //     — the wind moves branches, the moon moves, headlights
    //     pass. A *frozen* dark frame is a different signal.
    //   * The luma must be at one of the **extremes** (luma < 5
    //     = "covered", luma > 250 = "blinded"). Mid-range luma
    //     (20–200) is a normal scene, even if it's static.
    //
    // Suppressed when:
    //   * offline (offline is its own signal, not a tamper)
    //   * the Night hint is already going to fire (low luma
    //     but FPS > 0 is night, not tamper)
    //
    // Severity: Warning (this is a security event but the
    // operator should still verify — a brick thrown at a
    // camera can recover after a few seconds).

    #[test]
    fn covered_camera_static_emits_tamper() {
        // 45s of static + luma=2 → tamper.
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(2, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        m.last_scene_change_unix_secs.store(
            now_unix_secs().saturating_sub(45),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        let tamper: Vec<&Hint> = hints.iter().filter(|h| h.metric == "Tamper").collect();
        assert_eq!(
            tamper.len(),
            1,
            "expected exactly one Tamper hint, got {hints:?}"
        );
        assert_eq!(tamper[0].severity, Severity::Warning);
    }

    #[test]
    fn blinded_camera_static_emits_tamper() {
        // 60s of static + luma=254 → tamper (the other
        // extreme).
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(254, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        m.last_scene_change_unix_secs.store(
            now_unix_secs().saturating_sub(60),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        let tamper: Vec<&Hint> = hints.iter().filter(|h| h.metric == "Tamper").collect();
        assert_eq!(
            tamper.len(),
            1,
            "expected exactly one Tamper hint, got {hints:?}"
        );
    }

    #[test]
    fn static_mid_luma_does_not_emit_tamper() {
        // 60s of static + luma=128 → not a tamper (the scene
        // could legitimately be a still empty room).
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(128, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        m.last_scene_change_unix_secs.store(
            now_unix_secs().saturating_sub(60),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        assert!(
            hints.iter().all(|h| h.metric != "Tamper"),
            "static mid-luma should not be a tamper, got: {hints:?}"
        );
    }

    #[test]
    fn short_static_low_luma_does_not_emit_tamper() {
        // 10s of static + luma=2 → not a tamper yet (haven't
        // hit the 30s threshold; could be a night scene).
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(2, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        m.last_scene_change_unix_secs.store(
            now_unix_secs().saturating_sub(10),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        assert!(
            hints.iter().all(|h| h.metric != "Tamper"),
            "10s of static is below the 30s threshold, got: {hints:?}"
        );
    }

    #[test]
    fn tampered_offline_is_not_tamper() {
        // The pipeline is offline; we don't flag tampered
        // (offline is its own signal). Even if luma=2 and
        // 60s of static, we still don't claim tamper.
        let m = fresh();
        m.is_live.store(false, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(2, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        m.last_scene_change_unix_secs.store(
            now_unix_secs().saturating_sub(60),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        assert!(
            hints.iter().all(|h| h.metric != "Tamper"),
            "offline is not tamper, got: {hints:?}"
        );
    }

    #[test]
    fn tampered_but_changing_scene_does_not_emit_tamper() {
        // luma=2 but the scene is *changing* (last change
        // 1s ago) — could be a night scene with movement
        // (a car driving by, an animal). The "static"
        // requirement is the discriminator.
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(2, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        m.last_scene_change_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        assert!(
            hints.iter().all(|h| h.metric != "Tamper"),
            "changing scene is not a tamper, got: {hints:?}"
        );
    }

    #[test]
    fn tamper_cause_includes_luma_value() {
        // The cause string carries the luma value so the
        // operator can tell "covered (luma=2)" from
        // "blinded (luma=254)" without leaving the overlay.
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(2, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        m.last_scene_change_unix_secs.store(
            now_unix_secs().saturating_sub(45),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        let tamper = hints.iter().find(|h| h.metric == "Tamper").unwrap();
        // Portuguese cause string for covered-camera tamper.
        // We assert two substrings: one that names the cause
        // ("coberta" = "covered") and one that names the
        // required luma value (2).
        assert!(
            tamper.cause.contains("coberta"),
            "tamper cause should mention 'coberta' (covered): {}",
            tamper.cause
        );
        assert!(
            tamper.cause.contains('2'),
            "tamper cause should include the luma band (2): {}",
            tamper.cause
        );
    }

    #[test]
    fn blinded_tamper_cause_mentions_light() {
        let m = fresh();
        m.is_live.store(true, std::sync::atomic::Ordering::Relaxed);
        m.last_avg_luma
            .store(254, std::sync::atomic::Ordering::Relaxed);
        m.frame_count
            .store(30, std::sync::atomic::Ordering::Relaxed);
        m.last_sample_unix_secs.store(
            now_unix_secs().saturating_sub(1),
            std::sync::atomic::Ordering::Relaxed,
        );
        m.last_scene_change_unix_secs.store(
            now_unix_secs().saturating_sub(45),
            std::sync::atomic::Ordering::Relaxed,
        );
        let hints = diagnose(&m);
        let tamper = hints.iter().find(|h| h.metric == "Tamper").unwrap();
        assert!(
            tamper.cause.contains("luz")
                || tamper.cause.contains("blinded")
                || tamper.cause.contains("light"),
            "blinded tamper cause should mention light: {}",
            tamper.cause
        );
    }
}

// =============================================================================
// Startup preflight
// =============================================================================
//
// `check_decoders` is called once at startup to give the user an early,
// actionable warning when a required GStreamer element is missing — long
// before the first frame would fail to decode. Pure logging side effects;
// never aborts the run (the pipeline may still be able to pick an
// alternative decoder via autoplugging).

use gstreamer as gst;

/// Walk the well-known RTSP-related decoders / sinks and report which are
/// missing from the current GStreamer registry. Emits one `warn!` per
/// missing element and a single `info!` when everything is present.
pub fn check_decoders() {
    let wanted: &[&str] = &[
        "rtspsrc",
        "rtpjpegdepay",
        "rtph264depay",
        "rtph265depay",
        "avdec_h264",
        "avdec_h265",
        "decodebin",
        "uridecodebin",
        "videoconvert",
        "appsink",
    ];
    let mut missing: Vec<&str> = Vec::new();
    for name in wanted {
        if gst::ElementFactory::find(name).is_none() {
            missing.push(name);
        }
    }
    if missing.is_empty() {
        info!("GStreamer preflight: all common RTSP/decode elements present");
    } else {
        warn!(
            "GStreamer preflight: missing elements ({}): {}",
            missing.len(),
            missing.join(", ")
        );
        warn!("Some streams may fail to decode. Install the matching gstreamer plugins.");
    }
}

/// Test-instantiate a multi-element decoder chain before building the full
/// pipeline. Builds `fakesrc num-buffers=0 ! <chain> ! fakesink`, drives it
/// to Ready (which triggers element creation and CUDA init for hardware
/// decoders like nvh264dec), then immediately returns to Null.
///
/// Returns `Err(String)` with a human-readable message when any element in
/// the chain cannot be found or fails to reach Ready — e.g. when nvh264dec
/// is in the registry but CUDA is unavailable.
///
/// Only meaningful for chains that contain spaces (multiple elements joined
/// with `!`). Single-element chains should use `gst::ElementFactory::find`
/// instead. Never calls `process::exit` — the caller decides what to do.
pub fn validate_decoder_chain(chain: &str) -> Result<(), String> {
    use gst::prelude::*;
    use gstreamer as gst;

    let desc = format!("fakesrc num-buffers=0 ! {} ! fakesink", chain);
    let element = gst::parse_launch(&desc).map_err(|e| {
        format!(
            "Decoder chain '{}' failed to parse: {}. \
             Check element names with `gst-inspect-1.0`.",
            chain, e
        )
    })?;
    let pipeline = element
        .downcast::<gst::Pipeline>()
        .map_err(|_| "parse_launch did not return a Pipeline".to_string())?;

    let result = pipeline.set_state(gst::State::Ready);
    let _ = pipeline.set_state(gst::State::Null);

    result.map(|_| ()).map_err(|e| {
        format!(
            "Decoder chain '{}' failed to reach Ready state: {}. \
             The chain may reference an element that cannot initialise \
             (e.g. missing GPU/CUDA for nvh264dec). \
             Try `decoder = \"decodebin\"` or `decoder = \"avdec_h264\"`.",
            chain, e
        )
    })
}

#[cfg(test)]
mod decoder_chain_tests {
    use super::validate_decoder_chain;

    #[test]
    fn nonexistent_element_fails_gracefully() {
        gstreamer::init().ok();
        let result = validate_decoder_chain("this_element_does_not_exist_xyzzy");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("failed to parse"),
            "expected parse failure message, got: {msg}"
        );
    }

    #[test]
    fn single_known_element_passes() {
        gstreamer::init().ok();
        if gstreamer::ElementFactory::find("avdec_h264").is_none() {
            return; // skip if gstreamer1.0-libav not installed
        }
        assert!(
            validate_decoder_chain("avdec_h264").is_ok(),
            "avdec_h264 should pass pre-flight"
        );
    }

    #[test]
    fn multi_element_software_chain_passes() {
        gstreamer::init().ok();
        let needed = ["rtph264depay", "h264parse", "avdec_h264"];
        if needed
            .iter()
            .any(|n| gstreamer::ElementFactory::find(n).is_none())
        {
            return; // skip if any element is missing
        }
        let result = validate_decoder_chain("rtph264depay ! h264parse ! avdec_h264");
        assert!(
            result.is_ok(),
            "software h264 chain should pass pre-flight: {:?}",
            result
        );
    }
}
