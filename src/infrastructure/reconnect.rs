use log::{debug, info, warn};
use std::cell::Cell;
use std::time::Instant;

/// State for the FPS watchdog + backoff reconnect logic.
/// Shared between single-camera (App) and grid (CameraSlot) paths.
pub struct ReconnectState {
    fps_zero_since: Cell<Option<Instant>>,
    pub watchdog_stall_secs: u64,
}

impl ReconnectState {
    pub fn new(watchdog_stall_secs: u64) -> Self {
        Self {
            fps_zero_since: Cell::new(None),
            watchdog_stall_secs,
        }
    }

    /// Reset the stall timer. Called when a camera is toggled off.
    pub fn reset(&self) {
        self.fps_zero_since.set(None);
    }

    /// Check FPS watchdog and backoff timer. Returns `ReconnectDecision`
    /// indicating whether a reconnect was triggered.
    ///
    /// - `is_live`: whether the stream reports as live
    /// - `is_uridecodebin`: whether using uridecodebin (HLS — skip watchdog)
    /// - `current_fps`: current measured FPS (None if unavailable)
    /// - `backoff_due`: whether the backoff timer has expired
    /// - `label`: camera label for log messages
    pub fn tick(
        &self,
        is_live: bool,
        is_uridecodebin: bool,
        current_fps: Option<f64>,
        backoff_due: bool,
        label: &str,
    ) -> ReconnectDecision {
        // Backoff timer takes priority
        if backoff_due {
            info!("[{}] Backoff timer expired — triggering reconnect", label,);
            self.fps_zero_since.set(None);
            return ReconnectDecision::Reconnect("backoff timer");
        }

        // uridecodebin (HLS/DASH) is fully exempt from the watchdog: its
        // delivery is bursty, `is_live` is unreliable after a transient error,
        // and `reconnect_camera` runs synchronously on the UI thread — an
        // over-eager rebuild freezes every other camera. A dead HLS feed just
        // shows offline; only the backoff timer (armed by a prior reconnect)
        // can rebuild it.
        if is_uridecodebin {
            self.fps_zero_since.set(None);
            return ReconnectDecision::None;
        }

        // For RTSP only: a *live* stream whose decoder has stalled (FPS pinned
        // at 0) is the one case the watchdog acts on. `!is_live` on its own is
        // left alone — a bus error surfaces as an offline badge, not a rebuild
        // loop.
        let stalled = is_live && current_fps == Some(0.0);
        if !stalled {
            if self.fps_zero_since.get().is_some() {
                info!("[{}] Stream recovered", label);
            }
            self.fps_zero_since.set(None);
            return ReconnectDecision::None;
        }
        let reason = "watchdog stall";

        let since = match self.fps_zero_since.get() {
            Some(s) => s,
            None => {
                let now = Instant::now();
                self.fps_zero_since.set(Some(now));
                debug!("[{}] {} — starting recovery timer", label, reason);
                now
            }
        };

        let elapsed = since.elapsed().as_secs();
        if elapsed >= self.watchdog_stall_secs {
            warn!(
                "[{}] {} for {}s — forcing reconnect",
                label, reason, self.watchdog_stall_secs,
            );
            self.fps_zero_since.set(None);
            return ReconnectDecision::Reconnect(reason);
        } else if elapsed > 0 && elapsed % 5 == 0 {
            debug!(
                "[{}] {} for {}s (timeout at {}s)",
                label, reason, elapsed, self.watchdog_stall_secs,
            );
        }

        ReconnectDecision::None
    }
}

pub enum ReconnectDecision {
    None,
    Reconnect(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_backoff_due_returns_reconnect() {
        let rs = ReconnectState::new(15);
        let decision = rs.tick(true, false, Some(25.0), true, "cam");
        assert!(matches!(
            decision,
            ReconnectDecision::Reconnect("backoff timer")
        ));
    }

    #[test]
    fn test_fps_zero_starts_stall_timer() {
        let rs = ReconnectState::new(15);
        assert!(rs.fps_zero_since.get().is_none());
        rs.tick(true, false, Some(0.0), false, "cam");
        assert!(rs.fps_zero_since.get().is_some());
    }

    #[test]
    fn test_fps_zero_for_15s_triggers_reconnect() {
        let rs = ReconnectState::new(15);
        // Simulate that fps_zero_since was set 16 seconds ago
        rs.fps_zero_since
            .set(Some(Instant::now() - Duration::from_secs(16)));
        let decision = rs.tick(true, false, Some(0.0), false, "cam");
        assert!(matches!(
            decision,
            ReconnectDecision::Reconnect("watchdog stall")
        ));
        assert!(rs.fps_zero_since.get().is_none());
    }

    #[test]
    fn test_fps_recovery_clears_state() {
        let rs = ReconnectState::new(15);
        rs.tick(true, false, Some(0.0), false, "cam");
        assert!(rs.fps_zero_since.get().is_some());
        let decision = rs.tick(true, false, Some(25.0), false, "cam");
        assert!(matches!(decision, ReconnectDecision::None));
        assert!(rs.fps_zero_since.get().is_none());
    }

    #[test]
    fn test_uridecodebin_skips_watchdog() {
        let rs = ReconnectState::new(15);
        rs.fps_zero_since
            .set(Some(Instant::now() - Duration::from_secs(20)));
        let decision = rs.tick(true, true, Some(0.0), false, "cam");
        assert!(matches!(decision, ReconnectDecision::None));
        assert!(rs.fps_zero_since.get().is_none());
    }

    /// `!is_live` on its own (bus error, EOS) must NOT arm the watchdog:
    /// `reconnect_camera` runs synchronously on the UI thread, so an
    /// over-eager rebuild loop freezes the whole grid. A dead feed shows an
    /// offline badge instead.
    #[test]
    fn test_not_live_does_not_arm_watchdog() {
        let rs = ReconnectState::new(15);
        let decision = rs.tick(false, false, Some(0.0), false, "cam");
        assert!(matches!(decision, ReconnectDecision::None));
        assert!(rs.fps_zero_since.get().is_none());
    }

    #[test]
    fn test_not_live_for_15s_still_does_not_reconnect() {
        let rs = ReconnectState::new(15);
        rs.fps_zero_since
            .set(Some(Instant::now() - Duration::from_secs(60)));
        let decision = rs.tick(false, false, Some(0.0), false, "cam");
        assert!(matches!(decision, ReconnectDecision::None));
        assert!(rs.fps_zero_since.get().is_none());
    }

    /// HLS never triggers a watchdog reconnect, live or not.
    #[test]
    fn test_uridecodebin_not_live_does_not_reconnect() {
        let rs = ReconnectState::new(15);
        rs.fps_zero_since
            .set(Some(Instant::now() - Duration::from_secs(60)));
        let decision = rs.tick(false, true, Some(0.0), false, "cam");
        assert!(matches!(decision, ReconnectDecision::None));
    }

    #[test]
    fn test_live_stream_with_frames_is_healthy() {
        let rs = ReconnectState::new(15);
        rs.fps_zero_since
            .set(Some(Instant::now() - Duration::from_secs(60)));
        let decision = rs.tick(true, false, Some(25.0), false, "cam");
        assert!(matches!(decision, ReconnectDecision::None));
        assert!(rs.fps_zero_since.get().is_none());
    }
}
