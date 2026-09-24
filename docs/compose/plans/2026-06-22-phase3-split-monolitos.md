# Phase 3: Split Monolitos — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split CameraSlot (928 lines) and nvr/mod.rs (1489 lines) into focused modules with single responsibilities (SRP).

**Architecture:** Extract pipeline management, UI state, tick logic, and layout into separate modules. Each module has one clear responsibility.

**Tech Stack:** Rust 2024, GStreamer 0.20, GTK 0.18

---

### Task 1: Split nvr/mod.rs — Extract layout.rs

**Covers:** [S5.2]

**Files:**
- Create: `src/infrastructure/nvr/layout.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Create layout.rs with calc_grid and arrange**

Move `calc_grid()` and `arrange()` functions from `nvr/mod.rs` to `nvr/layout.rs`:

```rust
// src/infrastructure/nvr/layout.rs
use super::{CameraCell, LayoutMode};
use crate::infrastructure::camera::Camera;

pub(crate) fn calc_grid(n: usize) -> (i32, i32) {
    match n {
        0 | 1 => (1, 1),
        2 => (2, 1),
        3 | 4 => (2, 2),
        5 | 6 => (3, 2),
        7 | 8 | 9 => (3, 3),
        _ => {
            let cols = (n as f64).sqrt().ceil() as i32;
            (cols, (n as i32 + cols - 1) / cols)
        }
    }
}

pub(crate) fn arrange(
    grid_cont: &gtk::Grid,
    flex_cont: &gtk::Box,
    cell_data: &[CameraCell],
    active: &[bool],
    mode: LayoutMode,
    flex_main_idx: &std::cell::Cell<usize>,
    status_bar: &gtk::Label,
) {
    // ... existing arrange logic ...
}
```

- [ ] **Step 2: Update nvr/mod.rs to use layout module**

Add `mod layout;` and replace calls to `calc_grid` and `arrange` with `layout::calc_grid` and `layout::arrange`.

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -10`

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/layout.rs src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract layout logic to dedicated module"
```

---

### Task 2: Split nvr/mod.rs — Extract cell.rs

**Covers:** [S5.2]

**Files:**
- Create: `src/infrastructure/nvr/cell.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Create cell.rs with CameraCell struct and builder**

Move `CameraCell` struct, `build_cells()` function, and `create_cameras()` function from `nvr/mod.rs` to `nvr/cell.rs`.

- [ ] **Step 2: Update nvr/mod.rs to use cell module**

Add `mod cell;` and update references.

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -10`

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/cell.rs src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract CameraCell and cell builders to dedicated module"
```

---

### Task 3: Split nvr/mod.rs — Extract sidebar.rs

**Covers:** [S5.2]

**Files:**
- Create: `src/infrastructure/nvr/sidebar.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Create sidebar.rs with SidebarRow struct and builder**

Move `SidebarRow` struct and `build_sidebar()` function from `nvr/mod.rs` to `nvr/sidebar.rs`.

- [ ] **Step 2: Update nvr/mod.rs to use sidebar module**

Add `mod sidebar;` and update references.

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -10`

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/sidebar.rs src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract SidebarRow and sidebar builder to dedicated module"
```

---

### Task 4: Split nvr/mod.rs — Extract info_panel.rs

**Covers:** [S5.2]

**Files:**
- Create: `src/infrastructure/nvr/info_panel.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Create info_panel.rs with InfoPanel struct**

Move `InfoPanel` struct and its `new()` method from `nvr/mod.rs` to `nvr/info_panel.rs`.

- [ ] **Step 2: Update nvr/mod.rs to use info_panel module**

Add `mod info_panel;` and update references.

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -10`

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/info_panel.rs src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract InfoPanel to dedicated module"
```

---

### Task 5: Split nvr/mod.rs — Extract tick.rs

**Covers:** [S5.2]

**Files:**
- Create: `src/infrastructure/nvr/tick.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Create tick.rs with TickContext and TickContextRef**

Move `TickContext`, `TickContextRef` structs and their impls from `nvr/mod.rs` to `nvr/tick.rs`.

- [ ] **Step 2: Update nvr/mod.rs to use tick module**

Add `mod tick;` and update references.

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -10`

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/tick.rs src/infrastructure/nvr/mod.rs
git commit -m "refactor(nvr): extract tick logic to dedicated module"
```

---

### Task 6: Add tests for split modules

**Covers:** [S5.3]

**Files:**
- Modify: `src/infrastructure/nvr/layout.rs` (add tests)

- [ ] **Step 1: Add tests for calc_grid**

```rust
// Add to layout.rs #[cfg(test)] mod tests
#[test]
fn test_calc_grid_1() { assert_eq!(calc_grid(1), (1, 1)); }
#[test]
fn test_calc_grid_4() { assert_eq!(calc_grid(4), (2, 2)); }
#[test]
fn test_calc_grid_9() { assert_eq!(calc_grid(9), (3, 3)); }
#[test]
fn test_calc_grid_12() { assert_eq!(calc_grid(12), (4, 3)); }
```

- [ ] **Step 2: Run tests**

Run: `cargo test 2>&1 | grep "test result"`

- [ ] **Step 3: Commit**

```bash
git add src/infrastructure/nvr/layout.rs
git commit -m "test(nvr): add tests for calc_grid layout function"
```

---

### Task 7: Final verification

**Covers:** [S7]

**Files:** None (verification only)

- [ ] **Step 1: Full build**

Run: `cargo build 2>&1`
Expected: No errors.

- [ ] **Step 2: Full test suite**

Run: `cargo test 2>&1`
Expected: All tests pass (302+ including new ones).

- [ ] **Step 3: Check line counts**

Run: `wc -l src/infrastructure/nvr/*.rs`
Expected: `nvr/mod.rs` should be under 500 lines.

- [ ] **Step 4: Commit if any cleanup needed**

```bash
git add -u
git commit -m "refactor(nvr): complete monolith split phase"
```

---

## Summary

| Task | What | Lines moved | Risk |
|------|------|:-----------:|:----:|
| 1 | Extract layout.rs | ~50 | Low |
| 2 | Extract cell.rs | ~150 | Medium |
| 3 | Extract sidebar.rs | ~150 | Medium |
| 4 | Extract info_panel.rs | ~100 | Low |
| 5 | Extract tick.rs | ~300 | Medium |
| 6 | Tests | ~20 | Low |
| 7 | Final verification | 0 | None |

**Net result:** `nvr/mod.rs` drops from ~1489 to ~400-500 lines. Each module has one clear responsibility.
