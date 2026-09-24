# Phase 2: Polimorfismo — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Camera enum with CameraSource trait for OCP compliance, add OverlayRenderer and LayoutStrategy traits for extensibility.

**Architecture:** Define traits in domain, implement for each camera type, replace enum dispatch with trait objects.

**Tech Stack:** Rust 2024, GStreamer 0.20, GTK 0.18

---

### Task 1: Create CameraSource trait

**Covers:** [S4.1]

**Files:**
- Create: `src/domain/camera_source.rs`
- Modify: `src/domain/mod.rs`

- [ ] **Step 1: Create the trait**

```rust
// src/domain/camera_source.rs
use std::sync::Arc;
use crate::domain::metrics::Metrics;

pub trait CameraSource: std::fmt::Debug + Send + Sync {
    fn label(&self) -> &str;
    fn metrics(&self) -> Arc<Metrics>;
    fn is_live(&self) -> bool;
    fn audio_volume(&self) -> Option<f32>;
}
```

- [ ] **Step 2: Wire into domain/mod.rs**

Add `pub mod camera_source;` to `src/domain/mod.rs`.

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -10`

- [ ] **Step 4: Commit**

```bash
git add src/domain/camera_source.rs src/domain/mod.rs
git commit -m "feat(domain): add CameraSource trait for OCP compliance"
```

---

### Task 2: Implement CameraSource for CameraSlot

**Covers:** [S4.1]

**Files:**
- Modify: `src/infrastructure/camera_slot.rs`

- [ ] **Step 1: Implement CameraSource for CameraSlot**

```rust
// Add to camera_slot.rs
impl crate::domain::camera_source::CameraSource for CameraSlot {
    fn label(&self) -> &str {
        &self.label
    }
    fn metrics(&self) -> Arc<Metrics> {
        self.metrics.clone()
    }
    fn is_live(&self) -> bool {
        self.metrics.is_live.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn audio_volume(&self) -> Option<f32> {
        self.configured_volume
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build 2>&1 | head -10`

- [ ] **Step 3: Commit**

```bash
git add src/infrastructure/camera_slot.rs
git commit -m "feat: implement CameraSource trait for CameraSlot"
```

---

### Task 3: Create OverlayRenderer trait

**Covers:** [S4.2]

**Files:**
- Create: `src/domain/overlay_renderer.rs`
- Modify: `src/domain/mod.rs`

- [ ] **Step 1: Create the trait**

```rust
// src/domain/overlay_renderer.rs
use crate::domain::overlay::OverlayState;
use crate::domain::diagnostics::Hint;
use crate::domain::metrics::Metrics;

pub trait OverlayRenderer {
    fn render(&self, metrics: &Metrics, hints: &[Hint]) -> OverlayState;
}
```

- [ ] **Step 2: Wire into domain/mod.rs**

Add `pub mod overlay_renderer;` to `src/domain/mod.rs`.

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -10`

- [ ] **Step 4: Commit**

```bash
git add src/domain/overlay_renderer.rs src/domain/mod.rs
git commit -m "feat(domain): add OverlayRenderer trait for layout extensibility"
```

---

### Task 4: Create LayoutStrategy trait

**Covers:** [S4.3]

**Files:**
- Create: `src/domain/layout_strategy.rs`
- Modify: `src/domain/mod.rs`

- [ ] **Step 1: Create the trait**

```rust
// src/domain/layout_strategy.rs
pub trait LayoutStrategy {
    fn arrange(&self, n_cameras: usize) -> (i32, i32);
    fn name(&self) -> &str;
}
```

- [ ] **Step 2: Wire into domain/mod.rs**

Add `pub mod layout_strategy;` to `src/domain/mod.rs`.

- [ ] **Step 3: Verify compilation**

Run: `cargo build 2>&1 | head -10`

- [ ] **Step 4: Commit**

```bash
git add src/domain/layout_strategy.rs src/domain/mod.rs
git commit -m "feat(domain): add LayoutStrategy trait for grid/flex extensibility"
```

---

### Task 5: Add tests for new traits

**Covers:** [S4.4]

**Files:**
- Modify: `src/domain/camera_source.rs` (add tests)
- Modify: `src/domain/overlay_renderer.rs` (add tests)
- Modify: `src/domain/layout_strategy.rs` (add tests)

- [ ] **Step 1: Add tests for CameraSource**

```rust
// Add to camera_source.rs #[cfg(test)] mod tests
use std::sync::Arc;
use crate::domain::metrics::Metrics;

#[derive(Debug)]
struct MockCameraSource {
    label: String,
    live: bool,
}

impl CameraSource for MockCameraSource {
    fn label(&self) -> &str { &self.label }
    fn metrics(&self) -> Arc<Metrics> { Arc::new(Metrics::default()) }
    fn is_live(&self) -> bool { self.live }
    fn audio_volume(&self) -> Option<f32> { Some(0.8) }
}

#[test]
fn test_mock_camera_source_label() {
    let cam = MockCameraSource { label: "Test".into(), live: true };
    assert_eq!(cam.label(), "Test");
}

#[test]
fn test_mock_camera_source_is_live() {
    let cam = MockCameraSource { label: "Test".into(), live: true };
    assert!(cam.is_live());
}
```

- [ ] **Step 2: Add tests for LayoutStrategy**

```rust
// Add to layout_strategy.rs #[cfg(test)] mod tests

struct GridLayout;

impl LayoutStrategy for GridLayout {
    fn arrange(&self, n: usize) -> (i32, i32) {
        match n {
            0 | 1 => (1, 1),
            2 => (2, 1),
            3 | 4 => (2, 2),
            5 | 6 => (3, 2),
            _ => {
                let cols = (n as f64).sqrt().ceil() as i32;
                (cols, (n as i32 + cols - 1) / cols)
            }
        }
    }
    fn name(&self) -> &str { "grid" }
}

#[test]
fn test_grid_layout_1_camera() {
    let g = GridLayout;
    assert_eq!(g.arrange(1), (1, 1));
}

#[test]
fn test_grid_layout_4_cameras() {
    let g = GridLayout;
    assert_eq!(g.arrange(4), (2, 2));
}

#[test]
fn test_grid_layout_9_cameras() {
    let g = GridLayout;
    assert_eq!(g.arrange(9), (3, 3));
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test 2>&1 | grep "test result"`

- [ ] **Step 4: Commit**

```bash
git add src/domain/camera_source.rs src/domain/layout_strategy.rs
git commit -m "test(domain): add tests for CameraSource and LayoutStrategy traits"
```

---

### Task 6: Final verification

**Covers:** [S7]

**Files:** None (verification only)

- [ ] **Step 1: Full build**

Run: `cargo build 2>&1`
Expected: No errors.

- [ ] **Step 2: Full test suite**

Run: `cargo test 2>&1`
Expected: All tests pass (295+ including new ones).

- [ ] **Step 3: Verify domain purity**

Run: `grep -r "use crate::config" src/domain/`
Expected: No matches (domain has no infrastructure dependencies).

- [ ] **Step 4: Commit if any cleanup needed**

```bash
git add -u
git commit -m "refactor(domain): complete polimorfismo phase"
```

---

## Summary

| Task | What | Lines changed | Risk |
|------|------|:-------------:|:----:|
| 1 | CameraSource trait | ~20 | Low |
| 2 | Implement for CameraSlot | ~15 | Low |
| 3 | OverlayRenderer trait | ~15 | Low |
| 4 | LayoutStrategy trait | ~15 | Low |
| 5 | Tests | ~60 | Low |
| 6 | Final verification | 0 | None |

**Net result:** Domain traits enable OCP — new camera types, overlays, and layouts can be added without modifying existing code.
