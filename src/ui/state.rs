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
}

pub struct ContextMenu {
    pub camera_idx: usize,
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

    pub fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.next_attempt = None;
    }

    pub fn is_due(&self) -> bool {
        match self.next_attempt {
            Some(t) => Instant::now() >= t,
            None => false,
        }
    }
}

pub fn is_expired(toast: &Toast) -> bool {
    toast.shown_at.elapsed().as_secs() >= TOAST_DURATION_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(delay >= std::time::Duration::from_secs(1) - std::time::Duration::from_millis(50));
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
        };
        assert!(!is_expired(&t));
    }

    #[test]
    fn toast_expires_after_duration() {
        let t = Toast {
            message: "test".into(),
            shown_at: Instant::now() - std::time::Duration::from_secs(TOAST_DURATION_SECS + 1),
        };
        assert!(is_expired(&t));
    }

    #[test]
    fn context_menu_holds_camera_idx() {
        let cm = ContextMenu { camera_idx: 3 };
        assert_eq!(cm.camera_idx, 3);
    }
}
