use std::time::Instant;

pub const BACKOFF_INITIAL_SECS: u64 = 1;
pub const BACKOFF_MAX_SECS: u64 = 30;
/// Largest shift we ever feed to `2u64.pow(..)`. `2^5 = 32 > BACKOFF_MAX_SECS`,
/// so anything beyond this is already clamped to the ceiling anyway; capping the
/// exponent is what stops a long outage from overflowing (panic in debug, wrap
/// to 0 → reconnect storm in release).
const BACKOFF_MAX_SHIFT: u32 = 5;
/// Ceiling on `consecutive_failures` so a camera that stays down for hours does
/// not grow the counter without bound.
const MAX_CONSECUTIVE_FAILURES: u32 = 1_000_000;
pub const TOAST_DURATION_SECS: u64 = 3;
pub const VU_PEAK_DECAY_MS: u128 = 1200;

pub struct Toast {
    pub message: String,
    pub shown_at: Instant,
    /// Folder a click on the toast opens (snapshot saved, …).
    pub open_dir: Option<std::path::PathBuf>,
}

pub struct ContextMenu {
    pub camera_idx: usize,
    /// Where the menu opened. Captured once: reading the live pointer position
    /// at render time made the menu follow the mouse.
    pub anchor: iced::Point,
}

pub struct BackoffState {
    pub consecutive_failures: u32,
    pub next_attempt: Option<Instant>,
}

impl BackoffState {
    pub fn new() -> Self {
        Self {
            consecutive_failures: 0,
            next_attempt: None,
        }
    }

    pub fn record_failure(&mut self) {
        self.consecutive_failures = self
            .consecutive_failures
            .saturating_add(1)
            .min(MAX_CONSECUTIVE_FAILURES);
        let shift = (self.consecutive_failures - 1).min(BACKOFF_MAX_SHIFT);
        let delay_secs = BACKOFF_INITIAL_SECS
            .saturating_mul(2u64.pow(shift))
            .min(BACKOFF_MAX_SECS);
        self.next_attempt = Some(Instant::now() + std::time::Duration::from_secs(delay_secs));
        log::info!(
            "Backoff: attempt {} failed, next reconnect in {}s",
            self.consecutive_failures,
            delay_secs
        );
    }

    /// Arm the retry timer for a camera that is down but has none pending.
    ///
    /// A pipeline that errors *after* a successful start (camera offline at
    /// launch, playlist not ready, bus error) never reaches
    /// `reconnect_camera`'s failure path, so nothing else would ever schedule
    /// a retry. No-op while a retry is already pending.
    pub fn arm_if_idle(&mut self) {
        if self.next_attempt.is_none() {
            self.record_failure();
        }
    }

    /// Clear the pending retry after a rebuild was issued. The failure count is
    /// kept so the next delay keeps growing; leaving the expired timer armed
    /// would rebuild the pipeline on every tick.
    pub fn disarm(&mut self) {
        self.next_attempt = None;
    }

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.next_attempt = None;
    }

    /// Human-readable retry status for the tile placeholder, e.g.
    /// `tentativa 3 · próxima em 4s`. `None` while nothing has failed yet.
    pub fn status_detail(&self) -> Option<String> {
        if self.consecutive_failures == 0 {
            return None;
        }
        let attempt = self.consecutive_failures;
        Some(match self.next_attempt {
            Some(t) => {
                let secs = t
                    .saturating_duration_since(Instant::now())
                    .as_secs_f32()
                    .ceil() as u64;
                if secs == 0 {
                    format!("tentativa {attempt} · reconectando agora")
                } else {
                    format!("tentativa {attempt} · próxima em {secs}s")
                }
            }
            None => format!("tentativa {attempt}"),
        })
    }

    pub fn is_due(&self) -> bool {
        match self.next_attempt {
            Some(t) => Instant::now() >= t,
            None => false,
        }
    }
}

pub fn is_expired(toast: &Toast) -> bool {
    // Clickable toasts stay up longer so there is time to aim at them.
    let ttl = if toast.open_dir.is_some() {
        TOAST_DURATION_SECS * 2
    } else {
        TOAST_DURATION_SECS
    };
    toast.shown_at.elapsed().as_secs() >= ttl
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_detail_none_until_first_failure() {
        assert!(BackoffState::new().status_detail().is_none());
    }

    #[test]
    fn status_detail_reports_attempt_and_countdown() {
        let mut b = BackoffState::new();
        b.record_failure();
        b.record_failure();
        let d = b.status_detail().unwrap();
        assert!(d.starts_with("tentativa 2 · próxima em "), "{d}");
        b.next_attempt = Some(Instant::now() - std::time::Duration::from_secs(1));
        assert_eq!(
            b.status_detail().unwrap(),
            "tentativa 2 · reconectando agora"
        );
    }

    #[test]
    fn arm_if_idle_schedules_once_and_disarm_keeps_the_count() {
        let mut b = BackoffState::new();
        b.arm_if_idle();
        assert_eq!(b.consecutive_failures, 1);
        let first = b.next_attempt;
        assert!(first.is_some());
        b.arm_if_idle(); // already pending: untouched
        assert_eq!(b.consecutive_failures, 1);
        assert_eq!(b.next_attempt, first);
        b.disarm();
        assert!(!b.is_due());
        b.arm_if_idle();
        assert_eq!(b.consecutive_failures, 2);
    }

    #[test]
    fn backoff_initial_state() {
        let b = BackoffState::new();
        assert!(!b.is_due());
        assert_eq!(b.consecutive_failures, 0);
    }

    #[test]
    fn backoff_records_failure() {
        let mut b = BackoffState::new();
        b.record_failure();
        assert_eq!(b.consecutive_failures, 1);
        assert!(b.next_attempt.is_some());
    }

    #[test]
    fn backoff_is_due_after_delay() {
        let mut b = BackoffState::new();
        b.record_failure();
        b.next_attempt = Some(Instant::now() - std::time::Duration::from_secs(1));
        assert!(b.is_due());
    }

    #[test]
    fn backoff_records_success() {
        let mut b = BackoffState::new();
        b.record_failure();
        b.record_success();
        assert_eq!(b.consecutive_failures, 0);
        assert!(!b.is_due());
    }

    #[test]
    fn backoff_exponential_delay() {
        let mut b = BackoffState::new();
        b.record_failure();
        let t1 = b.next_attempt.unwrap();
        b.record_success();
        b.record_failure();
        let t2 = b.next_attempt.unwrap();
        let diff = if t1 > t2 { t1 - t2 } else { t2 - t1 };
        assert!(diff < std::time::Duration::from_millis(100));

        b.record_success();
        b.record_failure();
        b.record_failure();
        let t3 = b.next_attempt.unwrap();
        let delay = t3.duration_since(Instant::now());
        assert!(delay > std::time::Duration::from_secs(1));
        assert!(delay < std::time::Duration::from_secs(3));
    }

    #[test]
    fn backoff_capped_at_max() {
        let mut b = BackoffState::new();
        for _ in 0..10 {
            b.record_failure();
        }
        let delay = b.next_attempt.unwrap().duration_since(Instant::now());
        assert!(delay <= std::time::Duration::from_secs(BACKOFF_MAX_SECS + 1));
    }

    #[test]
    fn backoff_survives_prolonged_outage_without_overflow() {
        let mut b = BackoffState::new();
        // A camera down for hours reaches hundreds of failed attempts; the old
        // `2u64.pow(consecutive_failures - 1)` panicked (debug) / wrapped to a
        // zero delay (release) past ~64.
        for n in 1..=5_000u32 {
            b.record_failure();
            let delay = b.next_attempt.unwrap().duration_since(Instant::now());
            // Never longer than the ceiling, never a zero-delay storm.
            assert!(delay <= std::time::Duration::from_secs(BACKOFF_MAX_SECS + 1));
            assert!(
                delay >= std::time::Duration::from_secs(1) - std::time::Duration::from_millis(50)
            );
            // Once the exponential has climbed past the cap it stays pinned.
            if n > BACKOFF_MAX_SHIFT + 1 {
                assert!(delay >= std::time::Duration::from_secs(BACKOFF_MAX_SECS - 1));
            }
        }
        assert!(b.consecutive_failures <= MAX_CONSECUTIVE_FAILURES);
    }

    #[test]
    fn toast_not_expired_initially() {
        let t = Toast {
            message: "test".into(),
            shown_at: Instant::now(),
            open_dir: None,
        };
        assert!(!is_expired(&t));
    }

    #[test]
    fn toast_expires_after_duration() {
        let t = Toast {
            message: "test".into(),
            shown_at: Instant::now() - std::time::Duration::from_secs(TOAST_DURATION_SECS + 1),
            open_dir: None,
        };

        assert!(is_expired(&t));

        let clickable = Toast {
            open_dir: Some("/tmp".into()),
            ..t
        };
        assert!(!is_expired(&clickable));
    }

    #[test]
    fn context_menu_holds_camera_idx() {
        let cm = ContextMenu {
            camera_idx: 3,
            anchor: iced::Point::new(10.0, 20.0),
        };
        assert_eq!(cm.camera_idx, 3);
        assert_eq!(cm.anchor, iced::Point::new(10.0, 20.0));
    }
}
