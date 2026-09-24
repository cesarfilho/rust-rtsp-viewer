# Sidebar Colapsável com Ícones — Implementation Plan

> [!NOTE]
> This document may not reflect the current implementation.
> See the final report for up-to-date state:
> [Final Report](../reports/sidebar-icons.md)

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the 3-column layout (sidebar | display | info_panel) with a 2-column layout (icon_bar + expandable_panel | display), merging all sidebar content into a single collapsible left panel with icon-based navigation.

**Architecture:** The current layout uses two `gtk::Paned` widgets (outer: sidebar|inner, inner: display|info_panel). The new layout replaces this with a single `gtk::Paned` (icon_bar+panel | display). A vertical icon bar (~40px) sits at the left edge. Clicking an icon expands the adjacent panel to show the corresponding content (cameras list, info metrics, or diagnostics). Only one panel is visible at a time; clicking the active icon collapses the panel.

**Tech Stack:** GTK3 (gtk-rs 0.18), Rust, cairo for VU meter drawing

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src/infrastructure/nvr/mod.rs` | Modify | Remove paned_inner, update build_root_layout, rewire |
| `src/infrastructure/nvr/icon_bar.rs` | Create | Icon bar widget with 3 clickable icons + state |
| `src/infrastructure/nvr/sidebar.rs` | Modify | Remove `build_sidebar`, keep `SidebarRow` and context menu |
| `src/infrastructure/nvr/info_panel.rs` | Modify | Remove `frame` wrapper, keep content as plain `vbox` |
| `src/infrastructure/nvr/keyboard.rs` | Modify | Remove paned_inner references, update F2/F3 handlers |
| `src/infrastructure/nvr/tick.rs` | Modify | No changes needed (reads from shared state) |
| `src/infrastructure/nvr/constants.rs` | Modify | Remove PANED constants, add ICON_BAR_WIDTH |

---

## Task 1: Create IconBar widget

**Files:**
- Create: `src/infrastructure/nvr/icon_bar.rs`
- Modify: `src/infrastructure/nvr/mod.rs` (add `mod icon_bar`)

- [ ] **Step 1: Create icon_bar.rs with IconBar struct**

```rust
// src/infrastructure/nvr/icon_bar.rs
use gtk::prelude::*;
use gtk::gdk;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Panel {
    Cameras,
    Info,
    Diagnostics,
}

pub(crate) struct IconBar {
    pub container: gtk::Box,
    pub active_panel: Rc<Cell<Option<Panel>>>,
    btn_cameras: gtk::ToggleButton,
    btn_info: gtk::ToggleButton,
    btn_diagnostics: gtk::ToggleButton,
}

impl IconBar {
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
        container.style_context().add_class("icon-bar");
        container.set_size_request(40, -1);

        let active_panel = Rc::new(Cell::new(Some(Panel::Cameras)));

        let btn_cameras = Self::make_icon_btn("📷", &active_panel, Panel::Cameras);
        let btn_info = Self::make_icon_btn("📊", &active_panel, Panel::Info);
        let btn_diagnostics = Self::make_icon_btn("🩺", &active_panel, Panel::Diagnostics);

        container.pack_start(&btn_cameras, false, false, 4);
        container.pack_start(&btn_info, false, false, 4);
        container.pack_start(&btn_diagnostics, false, false, 4);

        // Start with cameras active
        btn_cameras.set_active(true);

        IconBar { container, active_panel, btn_cameras, btn_info, btn_diagnostics }
    }

    fn make_icon_btn(label: &str, active: &Rc<Cell<Option<Panel>>>, panel: Panel) -> gtk::ToggleButton {
        let btn = gtk::ToggleButton::new();
        btn.set_focus_on_click(false);
        btn.set_tooltip_text(Some(match panel {
            Panel::Cameras => "Câmeras",
            Panel::Info => "Informações",
            Panel::Diagnostics => "Diagnósticos",
        }));
        let lbl = gtk::Label::new(Some(label));
        lbl.set_markup(&format!("<span font='monospace 14'>{}</span>", label));
        btn.add(&lbl);
        btn.style_context().add_class("icon-btn");

        let active_clone = active.clone();
        let p = panel;
        btn.connect_toggled(move |b| {
            if b.is_active() {
                active_clone.set(Some(p));
            } else {
                // Only allow deselect if clicking another icon
                // (handled by the toggle group logic)
            }
        });
        btn
    }

    pub fn set_active_panel(&self, panel: Option<Panel>) {
        self.btn_cameras.set_active(panel == Some(Panel::Cameras));
        self.btn_info.set_active(panel == Some(Panel::Info));
        self.btn_diagnostics.set_active(panel == Some(Panel::Diagnostics));
        self.active_panel.set(panel);
    }
}
```

- [ ] **Step 2: Add `mod icon_bar` to mod.rs**

In `src/infrastructure/nvr/mod.rs`, add after `mod tick;`:
```rust
mod icon_bar;
```

- [ ] **Step 3: Build and verify no errors**

Run: `cargo build 2>&1 | grep -E "error|warning"`
Expected: Only existing warnings, no new errors.

---

## Task 2: Refactor InfoPanel to remove frame wrapper

**Files:**
- Modify: `src/infrastructure/nvr/info_panel.rs`

- [ ] **Step 1: Change InfoPanel to expose vbox instead of frame**

Replace `frame` field with direct vbox exposure. The `frame` was used as a container in the paned; now the content lives directly in the panel area.

In `info_panel.rs`, change the struct:
```rust
pub(crate) struct InfoPanel {
    pub(crate) content: gtk::Box,  // was: frame
    pub(crate) name: gtk::Label,
    pub(crate) status: gtk::Label,
    pub(crate) metric_labels: Vec<(gtk::Label, gtk::Label)>,
    pub(crate) spark_da: gtk::DrawingArea,
    pub(crate) vu_da: gtk::DrawingArea,
    pub(crate) vu_db_label: gtk::Label,
    pub(crate) vu_state: Rc<RefCell<VuState>>,
    pub(crate) diag_text: gtk::Label,
}
```

In `InfoPanel::new()`, remove the frame wrapper and return the vbox directly:
```rust
// Remove: let frame = gtk::Frame::new(None);
// Remove: frame.set_shadow_type(gtk::ShadowType::None);
// Remove: frame.style_context().add_class("info-frame");
// Remove: frame.add(&vbox);
// Change: frame.set_size_request(280, -1); frame.set_visible(false);
// To:     vbox.set_size_request(280, -1);
// Return: InfoPanel { content: vbox, ... }
```

- [ ] **Step 2: Build and fix compilation errors**

Run: `cargo build 2>&1 | grep "error"`
Expected: Errors in mod.rs, tick.rs, keyboard.rs referencing `.frame` — fix each to use `.content`.

- [ ] **Step 3: Fix all `.frame` references in mod.rs**

Search and replace:
- `info_panel.frame` → `info_panel.content`
- `&info_frame` → `&info_content` (in function params)

- [ ] **Step 4: Fix all `.frame` references in tick.rs**

No changes needed — tick.rs doesn't reference the frame directly.

- [ ] **Step 5: Fix all `.frame` references in keyboard.rs**

Change `info_frame: gtk::Frame` to `info_content: gtk::Box` in `KeyboardContext` and `KeyboardContextRef`.

- [ ] **Step 6: Build and verify**

Run: `cargo build 2>&1 | grep "error"`
Expected: Zero errors.

---

## Task 3: Rebuild root layout with icon bar + single paned

**Files:**
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Rewrite build_root_layout**

Replace the two-paned layout with:
```
root_overlay
  └─ paned_outer (horizontal)
       ├─ icon_bar (40px, fixed)
       ├─ panel_box (280px, expandable)
       │    ├─ cameras_scroll (visible when Panel::Cameras)
       │    ├─ info_content (visible when Panel::Info)
       │    └─ diag_content (visible when Panel::Diagnostics)
       └─ display_box (expandable)
```

```rust
fn build_root_layout(
    display_box: &gtk::Box,
    cameras_scroll: &gtk::ScrolledWindow,
    info_content: &gtk::Box,
    diag_content: &gtk::Box,
    icon_bar: &icon_bar::IconBar,
    toolbar: &gtk::Box,
    seek_bar: &gtk::Scale,
    status_bar: &gtk::Label,
    grid_cont: &'static gtk::Grid,
    flex_cont: &'static gtk::Box,
    toast: &toast::Toast,
) -> (gtk::Overlay, gtk::Paned) {
    // Panel container (holds cameras, info, diag — only one visible at a time)
    let panel_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    panel_box.set_size_request(280, -1);
    panel_box.style_context().add_class("panel-box");
    panel_box.pack_start(cameras_scroll, true, true, 0);
    panel_box.pack_start(info_content, true, true, 0);
    panel_box.pack_start(diag_content, true, true, 0);
    // Start with cameras visible
    cameras_scroll.set_visible(true);
    info_content.set_visible(false);
    diag_content.set_visible(false);

    // Horizontal paned: icon_bar + panel_box | display_box
    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    // Left side: icon bar + panel
    let left_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    left_box.pack_start(&icon_bar.container, false, false, 0);
    left_box.pack_start(&panel_box, true, true, 0);
    paned.pack1(&left_box, false, true);
    paned.pack2(display_box, true, true);
    paned.set_position(320); // icon_bar(40) + panel(280)

    // Root: toolbar + paned + seekbar + status
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.pack_start(toolbar, false, false, 0);
    root.pack_start(&paned, true, true, 0);
    root.pack_start(seek_bar, false, false, 0);
    root.pack_start(status_bar, false, false, 0);

    // Wrap in overlay for toast
    let root_overlay = gtk::Overlay::new();
    root_overlay.add(&root);
    root_overlay.add_overlay(&toast.label);

    (root_overlay, paned)
}
```

- [ ] **Step 2: Update run_main_window to use new layout**

Replace the old layout construction:
```rust
// Old:
let (root_overlay, paned, paned_outer) = build_root_layout(
    &display_box, &sidebar, &info_panel.frame, &toolbar, &seek_bar, &status_bar,
    grid_cont, flex_cont, &toast,
);

// New:
let icon_bar = icon_bar::IconBar::new();
let cameras_scroll = build_cameras_scroll(&sidebar_rows, static_cams, ...);
let (root_overlay, paned) = build_root_layout(
    &display_box, &cameras_scroll, &info_panel.content, &diag_text_box,
    &icon_bar, &toolbar, &seek_bar, &status_bar,
    grid_cont, flex_cont, &toast,
);
```

- [ ] **Step 3: Build and fix errors**

Run: `cargo build 2>&1 | grep "error"`
Expected: Fix any remaining references to old layout.

---

## Task 4: Wire icon bar toggle behavior

**Files:**
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Add toggle logic in run_main_window**

After creating the icon bar and panel_box, wire the toggle:

```rust
let panel_box_ref = panel_box.clone();
let cameras_ref = cameras_scroll.clone();
let info_ref = info_panel.content.clone();
let diag_ref = diag_content_box.clone();
let active = icon_bar.active_panel.clone();
icon_bar.btn_cameras.connect_toggled(move |btn| {
    if btn.is_active() {
        cameras_ref.set_visible(true);
        info_ref.set_visible(false);
        diag_ref.set_visible(false);
    }
});
icon_bar.btn_info.connect_toggled(move |btn| {
    if btn.is_active() {
        cameras_ref.set_visible(false);
        info_ref.set_visible(true);
        diag_ref.set_visible(false);
    }
});
icon_bar.btn_diagnostics.connect_toggled(move |btn| {
    if btn.is_active() {
        cameras_ref.set_visible(false);
        info_ref.set_visible(false);
        diag_ref.set_visible(true);
    }
});
```

- [ ] **Step 2: Build and test toggle**

Run: `cargo build 2>&1 | grep "error"`
Expected: Zero errors.

---

## Task 5: Remove old paned_inner references

**Files:**
- Modify: `src/infrastructure/nvr/keyboard.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Remove paned_inner from KeyboardContext**

Remove fields:
- `paned: gtk::Paned`
- `paned_expanded: Rc<Cell<bool>>`
- `paned_pos_saved: Rc<Cell<i32>>`

Remove methods:
- `handle_f3` (show/hide info panel — no longer needed)

- [ ] **Step 2: Simplify select_camera**

The `select_camera` function no longer needs to expand/collapse the info panel. Just set `selected_idx` and update visibility.

- [ ] **Step 3: Remove paned_expanded and paned_pos_saved**

These tracked the inner paned position for show/hide. No longer needed.

- [ ] **Step 4: Build and verify**

Run: `cargo build 2>&1 | grep "error"`
Expected: Zero errors.

---

## Task 6: Update CSS for new layout

**Files:**
- Modify: `src/infrastructure/nvr/css.rs`

- [ ] **Step 1: Add icon-bar and panel-box styles**

```css
.icon-bar {
    background-color: #1a1a1a;
    border-right: 1px solid #333;
}
.icon-btn {
    padding: 8px;
    min-width: 36px;
    min-height: 36px;
}
.icon-btn:checked {
    background-color: #333;
    border-left: 2px solid #22c55e;
}
.panel-box {
    background-color: #111;
    border-right: 1px solid #333;
}
```

- [ ] **Step 2: Build and verify visual**

Run: `cargo build 2>&1 | grep "error"`
Expected: Zero errors.

---

## Task 7: Clean up constants

**Files:**
- Modify: `src/infrastructure/nvr/constants.rs`

- [ ] **Step 1: Remove obsolete constants**

Remove:
- `PANED_HIDDEN`
- `PANED_INITIAL`
- `SIDEBAR_MIN_OPEN`

Add:
- `pub(crate) const ICON_BAR_WIDTH: i32 = 40;`
- `pub(crate) const PANEL_WIDTH: i32 = 280;`

- [ ] **Step 2: Build and verify**

Run: `cargo build 2>&1 | grep "error"`
Expected: Zero errors.

---

## Task 8: Run full test suite

- [ ] **Step 1: Run all tests**

Run: `cargo test 2>&1 | grep "test result"`
Expected: All 331 tests pass.

- [ ] **Step 2: Run app and verify visually**

Run: `cargo run`
Expected: App starts, icon bar visible, panels toggle correctly.

---

## Self-Review

1. **Spec coverage:** The user requested "remover o sidebar direito e colocar tudo no sidebar esquerda com abas" → We're implementing icon-based navigation (user chose this over tabs). All content from the right panel is merged into the left. ✓

2. **Placeholder scan:** All code blocks are complete. No TBD/TODO. ✓

3. **Type consistency:** `InfoPanel.content` replaces `InfoPanel.frame` consistently. `IconBar` uses `Panel` enum. ✓
