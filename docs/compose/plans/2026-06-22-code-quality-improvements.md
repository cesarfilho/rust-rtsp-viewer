# Code Quality Improvements — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Decompose the 1893-line `nvr/mod.rs` god module, eliminate 3x duplicated camera-selection logic, unify the duplicated reconnect state machine, clean up dead code, and extract magic numbers into named constants.

**Architecture:** Each phase is independently committable and testable. Phase 1-2 are pure refactors (no behavior change). Phase 3-4 clean up dead code and constants. Phase 5 adds test coverage.

**Tech Stack:** Rust 2024, GTK3 (`gtk 0.18`), GStreamer (`gstreamer 0.20`), `glib 0.18`

---

## File Structure (after refactoring)

```
src/infrastructure/nvr/
├── mod.rs              — run_main_window orchestrator only (~100 lines)
├── cell.rs             — CameraCell struct + builder
├── sidebar.rs          — SidebarRow struct + builder
├── info_panel.rs       — InfoPanel struct + update logic
├── keyboard.rs         — KeyboardContext + all handle_* methods
├── tick.rs             — TickContext + tick loop
├── toast.rs            — Toast struct
├── css.rs              — CSS_THEME constant
├── constants.rs        — All magic numbers as named constants
└── layout.rs           — Grid/flex arrangement logic

src/infrastructure/
├── camera_slot.rs      — minor: extract reconnect logic
├── reconnect.rs        — NEW: shared FPS watchdog + backoff state machine
└── nvr/mod.rs          — slimmed orchestrator
```

---

### Task 1: Extract `nvr/css.rs` — CSS theme constant

**Covers:** Cleanup (magic CSS inline)

**Files:**
- Create: `src/infrastructure/nvr/css.rs`
- Modify: `src/infrastructure/nvr/mod.rs` (remove CSS_THEME lines 45-98, add `mod css`)

- [ ] **Step 1: Create the CSS module**

```rust
// src/infrastructure/nvr/css.rs
pub(crate) const CSS_THEME: &str = r#"
/* paste the exact CSS from nvr/mod.rs lines 46-97 */
"#;
```

Copy the exact bytes from `nvr/mod.rs:46-97` (the `const CSS_THEME: &str` block).

- [ ] **Step 2: Wire into mod.rs**

In `src/infrastructure/nvr/mod.rs`, add at the top with the other `mod` declarations:

```rust
mod css;
```

Replace the reference `CSS_THEME` in the file with `css::CSS_THEME`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles with no errors.

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/css.rs src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract CSS theme to dedicated module"
```

---

### Task 2: Extract `nvr/constants.rs` — named constants

**Covers:** Cleanup (magic numbers)

**Files:**
- Create: `src/infrastructure/nvr/constants.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Create constants module**

```rust
// src/infrastructure/nvr/constants.rs

/// Paned position when info panel is collapsed (effectively hides it).
pub(crate) const PANED_HIDDEN: i32 = 99999;

/// Default paned position for the info panel sidebar.
pub(crate) const PANED_DEFAULT: i32 = 260;

/// Minimum paned position threshold to consider the panel "expanded".
pub(crate) const PANED_EXPANDED_THRESHOLD: i32 = 20;

/// Default paned position when toggling the sidebar open.
pub(crate) const PANED_MIN_OPEN: i32 = 60;

/// Sparkline buffer capacity (number of data points).
pub(crate) const SPARKLINE_BUF_CAP: usize = 64;

/// Tick interval in milliseconds.
pub(crate) const TICK_INTERVAL_MS: u32 = 250;
```

- [ ] **Step 2: Wire into mod.rs and replace magic numbers**

Add `mod constants;` to `nvr/mod.rs`.

Replace each occurrence:
- `self.paned.set_position(99999)` → `self.paned.set_position(constants::PANED_HIDDEN)`
- `self.paned.set_position(260)` or similar defaults → `constants::PANED_DEFAULT`
- `pos > 20` → `pos > constants::PANED_EXPANDED_THRESHOLD`
- `max(60)` → `max(constants::PANED_MIN_OPEN)`
- `VecDeque::with_capacity(64)` → `VecDeque::with_capacity(constants::SPARKLINE_BUF_CAP)`
- `timeout_add_local(250, ...)` → `timeout_add_local(constants::TICK_INTERVAL_MS, ...)`

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -20`

- [ ] **Step 4: Run tests**

Run: `cargo test 2>&1 | tail -5`
Expected: 282 passed.

- [ ] **Step 5: Commit**

```bash
git add src/infrastructure/nvr/constants.rs src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract magic numbers to named constants"
```

---

### Task 3: Extract `select_camera` helper — eliminate triple duplication

**Covers:** Duplication (wire_cell_clicks, wire_sidebar_clicks, handle_number)

**Files:**
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Write the shared `select_camera` function**

Insert before `wire_cell_clicks` (around line 1559):

```rust
/// Shared camera selection logic used by cell clicks, sidebar clicks,
/// and keyboard number keys. Stops any playing audio pipeline, updates
/// the selection state, shows the info panel, and starts audio for the
/// new camera if configured.
fn select_camera(
    idx: usize,
    selected_idx: &Cell<Option<usize>>,
    info_frame: &gtk::Frame,
    paned: &gtk::Paned,
    paned_expanded: &Cell<bool>,
    paned_pos_saved: &Cell<i32>,
    spark_da: &gtk::DrawingArea,
    audio_pipeline: &RefCell<Option<gst::Pipeline>>,
    audio_muted: &Cell<bool>,
    cell_data: &[CameraCell],
    static_cams: &'static [Camera],
    start_audio: &dyn Fn(&str, f32) -> Option<gst::Pipeline>,
    layout_mode: Option<(&Cell<LayoutMode>, &Cell<usize>, &gtk::Grid, &gtk::Box, &Cell<Vec<bool>>, &gtk::Label)>,
) {
    // Stop previous audio
    if let Some(prev) = selected_idx.get() {
        if let Some(prev_ce) = cell_data.get(prev) {
            prev_ce.audio_badge.hide();
        }
    }
    audio_muted.set(false);
    if let Some(ap) = audio_pipeline.borrow_mut().take() {
        if let Some(bus) = ap.bus() {
            let _ = bus.remove_watch();
        }
        let _ = ap.set_state(gst::State::Null);
    }

    // Select new camera
    selected_idx.set(Some(idx));
    info_frame.set_visible(true);
    paned.set_position(paned_pos_saved.get());
    paned_expanded.set(true);
    spark_da.queue_draw();

    // Start audio if configured
    if let Some(cam) = static_cams.get(idx) {
        if let Some(vol) = cam.audio_volume() {
            if let Some(ap) = start_audio(cam.url(), vol) {
                *audio_pipeline.borrow_mut() = Some(ap);
                if let Some(ce) = cell_data.get(idx) {
                    ce.audio_badge.show();
                }
            }
        }
    }

    // Flex layout: update main camera if switching
    if let Some((layout_mode, flex_main_idx, grid_cont, flex_cont, active, status_bar)) = layout_mode {
        if layout_mode.get() == LayoutMode::Flex && flex_main_idx.get() != idx && active.borrow()[idx] {
            flex_main_idx.set(idx);
            arrange(grid_cont, flex_cont, cell_data, &active.borrow(),
                LayoutMode::Flex, flex_main_idx, status_bar);
        }
    }
}
```

- [ ] **Step 2: Refactor `wire_cell_clicks` to use `select_camera`**

Replace the closure body in `wire_cell_clicks` (lines 1595-1623) with:

```rust
c.ev_box.connect_button_press_event(move |_, _| {
    select_camera(
        i,
        &sel,
        &inf,
        &pn,
        &exp,
        &pos,
        &spark_c,
        &ap_cell,
        &am_cell,
        &cell_data_c,
        static_cams,
        &*start_audio_cell,
        Some((&layout_c, &flex_c, grid_cont, flex_cont, &active_c, &status_c)),
    );
    glib::Propagation::Stop
});
```

- [ ] **Step 3: Refactor `wire_sidebar_clicks` to use `select_camera`**

Replace the closure body in `wire_sidebar_clicks` (lines 1653-1680) with:

```rust
sr._ev.connect_button_press_event(move |_, ev| {
    if ev.button() != 1 { return glib::Propagation::Proceed; }
    select_camera(
        idx,
        &sel,
        &inf,
        &pn,
        &exp,
        &pos,
        &spark2,
        &ap_side,
        &am_side,
        &cd_side,
        static_cams,
        &*start_audio_side,
        None,
    );
    glib::Propagation::Proceed
});
```

- [ ] **Step 4: Refactor `handle_number` to use `select_camera`**

Replace the body of `handle_number` (lines 621-647) with:

```rust
select_camera(
    idx,
    &self.selected_idx,
    &self.info_frame,
    &self.paned,
    &self.paned_expanded,
    &self.paned_pos_saved,
    &self.spark_da,
    &self.audio_pipeline,
    &self.audio_muted,
    &self.cell_data,
    self.static_cams,
    &*self.start_audio,
    None,
);
```

- [ ] **Step 5: Verify compilation**

Run: `cargo build 2>&1 | head -20`

- [ ] **Step 6: Run tests**

Run: `cargo test 2>&1 | tail -5`
Expected: 282 passed.

- [ ] **Step 7: Commit**

```bash
git add src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract shared select_camera to eliminate 3x duplication"
```

---

### Task 4: Extract `reconnect.rs` — unified reconnect state machine

**Covers:** Duplication (FPS watchdog + backoff in App and CameraSlot)

**Files:**
- Create: `src/infrastructure/reconnect.rs`
- Modify: `src/infrastructure/mod.rs` (add `mod reconnect`)
- Modify: `src/infrastructure/camera_slot.rs` (use shared logic)
- Modify: `src/infrastructure/gtk_app/mod.rs` (use shared logic)

- [ ] **Step 1: Create the shared reconnect module**

```rust
// src/infrastructure/reconnect.rs
use std::time::{Duration, Instant};
use log::{debug, info, warn};

/// State for the FPS watchdog + backoff reconnect logic.
/// Shared between single-camera (App) and grid (CameraSlot) paths.
pub struct ReconnectState {
    pub fps_zero_since: Option<Instant>,
    pub watchdog_stall_secs: u64,
}

impl ReconnectState {
    pub fn new(watchdog_stall_secs: u64) -> Self {
        Self {
            fps_zero_since: None,
            watchdog_stall_secs,
        }
    }

    /// Check FPS watchdog and backoff timer. Returns `true` if a reconnect
    /// was triggered (caller should initiate pipeline restart).
    ///
    /// - `is_live`: whether the stream reports as live
    /// - `is_uridecodebin`: whether using uridecodebin (HLS — skip watchdog)
    /// - `current_fps`: current measured FPS (None if unavailable)
    /// - `backoff_due`: whether the backoff timer has expired
    /// - `label`: camera label for log messages
    pub fn tick(
        &mut self,
        is_live: bool,
        is_uridecodebin: bool,
        current_fps: Option<f64>,
        backoff_due: bool,
        label: &str,
    ) -> bool {
        let now = Instant::now();

        // Backoff timer takes priority
        if backoff_due {
            info!(
                "[{}] Backoff timer expired — triggering reconnect",
                label,
            );
            self.fps_zero_since = None;
            return true;
        }

        // FPS watchdog
        if is_uridecodebin {
            self.fps_zero_since = None;
            return false;
        }

        match (is_live, current_fps) {
            (true, Some(0.0)) => {
                let since = self.fps_zero_since.get_or_insert(now);
                let elapsed = since.elapsed().as_secs();
                if elapsed >= self.watchdog_stall_secs {
                    warn!(
                        "[{}] FPS=0 for {}s — decoder stalled, forcing reconnect",
                        label, self.watchdog_stall_secs,
                    );
                    self.fps_zero_since = None;
                    return true;
                } else if elapsed > 0 && elapsed % 5 == 0 {
                    debug!(
                        "[{}] FPS=0 for {}s (timeout at {}s)",
                        label, elapsed, self.watchdog_stall_secs,
                    );
                }
            }
            (true, Some(_)) => {
                if self.fps_zero_since.is_some() {
                    info!("[{}] FPS recovered", label);
                }
                self.fps_zero_since = None;
            }
            _ => {
                self.fps_zero_since = None;
            }
        }
        false
    }
}
```

- [ ] **Step 2: Wire into `infrastructure/mod.rs`**

Add `pub(crate) mod reconnect;` to `src/infrastructure/mod.rs`.

- [ ] **Step 3: Refactor `CameraSlot` to use `ReconnectState`**

In `camera_slot.rs`:
- Add field `reconnect_state: reconnect::ReconnectState` (replaces `fps_zero_since` and `watchdog_stall_secs` fields)
- In `update_overlay` (around line 673), replace the FPS watchdog block with:

```rust
let backoff_due = self.next_reconnect_at.lock().ok()
    .and_then(|s| *s)
    .map(|t| now >= t)
    .unwrap_or(false);

if self.reconnect_state.tick(
    is_live,
    self.use_uridecodebin,
    Some(state.current_fps),
    backoff_due,
    &self.label,
) {
    self.trigger_reconnect("watchdog-or-backoff");
    return;
}
```

Remove the old `fps_zero_since` field and the manual watchdog/backoff logic.

- [ ] **Step 4: Refactor `App` tick to use `ReconnectState`**

In `gtk_app/mod.rs` tick closure (around line 1394):
- Add field `reconnect_state: reconnect::ReconnectState` to `App`
- Replace the entire watchdog+backoff block (lines 1394-1451) with:

```rust
let is_live = app_for_tick.metrics.is_live.load(Ordering::SeqCst);
let is_uridecodebin = app_for_tick.pipeline.by_name("uridecodebin0").is_some();
let backoff_due = app_for_tick.next_reconnect_at.lock().ok()
    .and_then(|slot| *slot)
    .map(|t| Instant::now() >= t)
    .unwrap_or(false);
let fps = app_for_tick.metrics.snapshot_current_fps();

if app_for_tick.reconnect_state.tick(
    is_live,
    is_uridecodebin,
    fps,
    backoff_due,
    "main",
) {
    app_for_tick.trigger_reconnect(&tuning_for_tick, "watchdog-or-backoff");
    state.fps_zero_since = None;
    state.warmup_complete = false;
    state.warmup_start = Instant::now();
    state.queue_high_streak = 0;
}
```

Note: `state.fps_zero_since` is now managed by `ReconnectState`, so remove the local field from `State` in `gtk_app/mod.rs`. The warmup reset is still needed locally.

- [ ] **Step 5: Verify compilation**

Run: `cargo build 2>&1 | head -30`

- [ ] **Step 6: Run tests**

Run: `cargo test 2>&1 | tail -5`
Expected: 282 passed.

- [ ] **Step 7: Commit**

```bash
git add src/infrastructure/reconnect.rs src/infrastructure/mod.rs \
        src/infrastructure/camera_slot.rs src/infrastructure/gtk_app/mod.rs
git commit -m "refactor: extract shared ReconnectState for FPS watchdog + backoff"
```

---

### Task 5: Clean up `#[allow(dead_code)]`

**Covers:** Dead code cleanup

**Files:**
- Multiple files (see below)

- [ ] **Step 1: Audit each `#[allow(dead_code)]`**

Review these 12 occurrences:
- `infrastructure/recording.rs:247` — check if the field/method is used
- `pipeline.rs:38, 318` — check if variants/methods are used
- `infrastructure/camera_slot.rs:61, 430` — check if struct/field is used
- `infrastructure/gtk_app/mod.rs:94, 412, 414, 439, 443` — check if Tuning fields are used
- `domain/diagnostics.rs:52, 400` — check if variants are used

For each:
- If used only in tests → keep `#[allow(dead_code)]` with a comment `// tested`
- If unused anywhere → remove the dead code entirely
- If used in production code → remove the `#[allow(dead_code)]`

- [ ] **Step 2: Remove or fix each occurrence**

Apply the decision from step 1 to each file.

- [ ] **Step 3: Verify compilation and tests**

Run: `cargo build 2>&1 | head -20 && cargo test 2>&1 | tail -5`

- [ ] **Step 4: Commit**

```bash
git add -u
git commit -m "chore: clean up dead code and remove unnecessary #[allow(dead_code)]"
```

---

### Task 6: Extract `nvr/toast.rs`

**Covers:** Decomposition (small struct extraction)

**Files:**
- Create: `src/infrastructure/nvr/toast.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Create Toast module**

Move the `Toast` struct and its `impl` block from `nvr/mod.rs` into a new file:

```rust
// src/infrastructure/nvr/toast.rs
use gtk::prelude::*;
use glib::clone;

pub(crate) struct Toast {
    pub label: gtk::Label,
}

impl Toast {
    pub fn new() -> Self {
        let label = gtk::Label::new(None);
        label.set_halign(gtk::Align::End);
        label.set_valign(gtk::Align::End);
        label.set_margin_end(12);
        label.set_margin_bottom(48);
        label.style_context().add_class("toast");
        Toast { label }
    }

    pub fn show(&self, text: &str) {
        self.label.set_text(text);
        self.label.show();
        let label = self.label.clone();
        glib::timeout_add_local(std::time::Duration::from_secs(3), move || {
            label.hide();
            glib::ControlFlow::Break
        });
    }
}
```

- [ ] **Step 2: Remove Toast from nvr/mod.rs and add mod toast**

Remove the `Toast` struct and `impl Toast` block from `mod.rs`.
Add `mod toast;` and use `toast::Toast` where needed.

- [ ] **Step 3: Verify compilation and tests**

Run: `cargo build 2>&1 | head -20 && cargo test 2>&1 | tail -5`

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/toast.rs src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract Toast struct to dedicated module"
```

---

### Task 7: Extract `nvr/keyboard.rs`

**Covers:** Decomposition (keyboard handling)

**Files:**
- Create: `src/infrastructure/nvr/keyboard.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Create keyboard module**

Move `KeyboardContext` struct, `KeyboardContextRef` struct, and all `impl KeyboardContextRef` methods (`dispatch`, `handle_m`, `handle_r`, `handle_s`, `handle_space`, `handle_enter`, `handle_escape`, `handle_f2`, `handle_f3`, `handle_tab`, `handle_number`, `handle_question`) from `nvr/mod.rs` into a new file.

The struct fields reference types from `mod.rs` (CameraCell, SidebarRow, etc.), so import them:

```rust
// src/infrastructure/nvr/keyboard.rs
use super::{CameraCell, SidebarRow, InfoPanel, Toast, LayoutMode, arrange, select_camera};
use crate::infrastructure::camera::Camera;
// ... other imports
```

- [ ] **Step 2: Remove from mod.rs and add mod keyboard**

Remove the `KeyboardContext`, `KeyboardContextRef` structs and all their impls from `mod.rs`.
Add `mod keyboard;` and update references to `keyboard::KeyboardContext` etc.

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -30`

- [ ] **Step 4: Run tests**

Run: `cargo test 2>&1 | tail -5`

- [ ] **Step 5: Commit**

```bash
git add src/infrastructure/nvr/keyboard.rs src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract keyboard handling to dedicated module"
```

---

### Task 8: Final cleanup — verify and run full test suite

**Covers:** Verification

**Files:** None (verification only)

- [ ] **Step 1: Full build**

Run: `cargo build 2>&1`
Expected: No errors.

- [ ] **Step 2: Full test suite**

Run: `cargo test 2>&1`
Expected: 282 passed, 0 failed.

- [ ] **Step 3: Check final line counts**

Run: `wc -l src/infrastructure/nvr/*.rs src/infrastructure/reconnect.rs`
Expected: `nvr/mod.rs` should be under 400 lines. Total across all nvr files should be ~1800-2000 (some overhead from mod declarations and imports).

- [ ] **Step 4: Final commit if any cleanup needed**

```bash
git add -u
git commit -m "refactor(nvr): complete decomposition — mod.rs under 400 lines"
```

---

## Summary

| Task | What | Lines removed from mod.rs | Risk |
|------|------|:-------------------------:|------|
| 1 | CSS → css.rs | ~55 | Low |
| 2 | Magic numbers → constants.rs | ~10 (but scattered) | Low |
| 3 | select_camera helper | ~60 (3 copies → 1) | Medium |
| 4 | ReconnectState | ~80 (duplicated) | Medium |
| 5 | Dead code cleanup | varies | Low |
| 6 | Toast → toast.rs | ~20 | Low |
| 7 | Keyboard → keyboard.rs | ~250 | Medium |
| 8 | Final verification | 0 | None |

**Net result:** `nvr/mod.rs` drops from 1893 lines to ~400 lines. Three copies of camera selection become one. Two parallel reconnect state machines become one shared implementation.
