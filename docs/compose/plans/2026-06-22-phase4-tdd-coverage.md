# Phase 4: TDD Coverage — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add comprehensive test coverage for ReconnectState, cache controller, overlay rendering, and layout calculations.

**Architecture:** Unit tests for pure domain logic and infrastructure controllers. No GTK/GStreamer mocking — test logic only.

**Tech Stack:** Rust 2024, GStreamer 0.20, GTK 0.18

---

### Task 1: Test ReconnectState

**Covers:** [S6.2]

**Files:**
- Modify: `src/infrastructure/reconnect.rs`

- [ ] **Step 1: Add tests for ReconnectState::tick()**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_due_returns_reconnect() {
        let rs = ReconnectState::new(15);
        let result = rs.tick(true, false, Some(25.0), true, "test");
        assert!(matches!(result, ReconnectDecision::Reconnect("backoff timer")));
    }

    #[test]
    fn test_fps_zero_starts_stall_timer() {
        let rs = ReconnectState::new(15);
        let result = rs.tick(true, false, Some(0.0), false, "test");
        assert!(matches!(result, ReconnectDecision::None));
        assert!(rs.fps_zero_since.get().is_some());
    }

    #[test]
    fn test_fps_zero_for_15s_returns_reconnect() {
        let rs = ReconnectState::new(15);
        rs.tick(true, false, Some(0.0), false, "test");
        // Simulate 15 seconds passing by setting fps_zero_since to past
        rs.fps_zero_since.set(Some(Instant::now() - Duration::from_secs(16)));
        let result = rs.tick(true, false, Some(0.0), false, "test");
        assert!(matches!(result, ReconnectDecision::Reconnect("watchdog stall")));
    }

    #[test]
    fn test_fps_recovery_clears_state() {
        let rs = ReconnectState::new(15);
        rs.tick(true, false, Some(0.0), false, "test");
        assert!(rs.fps_zero_since.get().is_some());
        rs.tick(true, false, Some(25.0), false, "test");
        assert!(rs.fps_zero_since.get().is_none());
    }

    #[test]
    fn test_uridecodebin_skips_watchdog() {
        let rs = ReconnectState::new(15);
        let result = rs.tick(true, true, Some(0.0), false, "test");
        assert!(matches!(result, ReconnectDecision::None));
        assert!(rs.fps_zero_since.get().is_none());
    }

    #[test]
    fn test_not_live_clears_state() {
        let rs = ReconnectState::new(15);
        rs.tick(true, false, Some(0.0), false, "test");
        assert!(rs.fps_zero_since.get().is_some());
        rs.tick(false, false, Some(0.0), false, "test");
        assert!(rs.fps_zero_since.get().is_none());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test reconnect -- --nocapture`
Expected: 6 tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/infrastructure/reconnect.rs
git commit -m "test: add unit tests for ReconnectState tick logic"
```

---

### Task 2: Test cache controller logic

**Covers:** [S6.2]

**Files:**
- Modify: `src/infrastructure/gtk_app/cache_controller.rs`

- [ ] **Step 1: Read cache_controller.rs to understand the logic**

The cache controller has `clamp_cache_delta` and `adjust_cache` functions. These are pure functions that can be tested.

- [ ] **Step 2: Add tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp_cache_delta_within_bounds() {
        let (min, max) = Tuning::test_defaults().clamp_cache_delta(10, 5);
        assert_eq!(min, 10);
        assert_eq!(max, 15);
    }

    #[test]
    fn test_clamp_cache_delta_at_max() {
        let (min, max) = Tuning::test_defaults().clamp_cache_delta(55, 10);
        assert_eq!(min, 55);
        assert_eq!(max, 60);
    }

    #[test]
    fn test_clamp_cache_delta_at_min() {
        let (min, max) = Tuning::test_defaults().clamp_cache_delta(2, -5);
        assert_eq!(min, 0);
        assert_eq!(max, 0);
    }
}
```

Note: The exact test code depends on the actual function signatures. Adapt as needed.

- [ ] **Step 3: Run tests**

Run: `cargo test cache_controller -- --nocapture`

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/gtk_app/cache_controller.rs
git commit -m "test: add unit tests for cache controller delta clamping"
```

---

### Task 3: Test overlay rendering

**Covers:** [S6.2]

**Files:**
- Modify: `src/domain/overlay.rs` (already has ~40 tests, add edge cases)

- [ ] **Step 1: Add edge case tests**

```rust
#[test]
fn test_overlay_empty_hints() {
    let m = Metrics::new();
    let hints = vec![];
    let state = OverlayState::new(&m, &hints, "host", "TCP", "decodebin");
    assert_eq!(state.health, crate::domain::diagnostics::Severity::Healthy);
}

#[test]
fn test_overlay_critical_hint() {
    let m = Metrics::new();
    let hints = vec![Hint::new("FPS", Severity::Critical, "decoder stalled")];
    let state = OverlayState::new(&m, &hints, "host", "TCP", "decodebin");
    assert_eq!(state.health, Severity::Critical);
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test overlay -- --nocapture`

- [ ] **Step 3: Commit**

```bash
git add src/domain/overlay.rs
git commit -m "test: add edge case tests for overlay rendering"
```

---

### Task 4: Test layout calculations

**Covers:** [S6.2]

**Files:**
- Modify: `src/infrastructure/nvr/layout.rs` (already has 6 tests, add more)

- [ ] **Step 1: Add edge case tests**

```rust
#[test]
fn test_calc_grid_0() { assert_eq!(calc_grid(0), (1, 1)); }

#[test]
fn test_calc_grid_7() { assert_eq!(calc_grid(7), (3, 3)); }

#[test]
fn test_calc_grid_16() { assert_eq!(calc_grid(16), (4, 4)); }

#[test]
fn test_calc_grid_25() { assert_eq!(calc_grid(25), (5, 5)); }
```

- [ ] **Step 2: Run tests**

Run: `cargo test layout -- --nocapture`

- [ ] **Step 3: Commit**

```bash
git add src/infrastructure/nvr/layout.rs
git commit -m "test: add edge case tests for layout calculations"
```

---

### Task 5: Final verification

**Covers:** [S6.3]

**Files:** None (verification only)

- [ ] **Step 1: Full build**

Run: `cargo build 2>&1`
Expected: No errors.

- [ ] **Step 2: Full test suite**

Run: `cargo test 2>&1`
Expected: All tests pass (308+ including new ones).

- [ ] **Step 3: Check test count**

Run: `cargo test 2>&1 | grep "test result"`

- [ ] **Step 4: Commit if any cleanup needed**

```bash
git add -u
git commit -m "test: complete TDD coverage phase"
```

---

## Summary

| Task | What | Tests added | Risk |
|------|------|:-----------:|:----:|
| 1 | ReconnectState tests | 6 | Low |
| 2 | Cache controller tests | 3 | Low |
| 3 | Overlay edge cases | 2 | Low |
| 4 | Layout edge cases | 4 | Low |
| 5 | Final verification | 0 | None |

**Net result:** ~15 new tests covering critical domain and infrastructure logic. Total tests: ~323.
