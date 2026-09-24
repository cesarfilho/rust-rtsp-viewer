# UI Design System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a professional design system for rust-rtsp-viewer with semantic color tokens, consistent typography, spacing scale, and improved component patterns.

**Architecture:** CSS custom properties for tokens, constants module for spacing values, incremental component updates following the design system.

**Tech Stack:** GTK3, CSS, Rust

---

## File Structure

| File | Responsibility |
|------|----------------|
| `src/infrastructure/nvr/css.rs` | CSS tokens, variables, component styles |
| `src/infrastructure/nvr/constants.rs` | Spacing scale constants |
| `src/infrastructure/nvr/cell.rs` | CameraCell component styling |
| `src/infrastructure/nvr/sidebar.rs` | Sidebar row styling |
| `src/infrastructure/nvr/info_panel.rs` | Info panel styling |
| `src/infrastructure/nvr/mod.rs` | Toolbar, layout styling |

---

### Task 1: CSS Tokens Foundation

**Covers:** [S3]

**Files:**
- Modify: `src/infrastructure/nvr/css.rs`

- [ ] **Step 1: Read current CSS**

Read `src/infrastructure/nvr/css.rs` to understand current structure.

- [ ] **Step 2: Add CSS custom properties**

Replace the hardcoded colors with CSS variables at the top of the CSS string:

```rust
pub(crate) const CSS_THEME: &str = ":root { \
    --bg-primary: #0a0a0a; \
    --bg-secondary: #0d0d0d; \
    --bg-tertiary: #141414; \
    --border-subtle: #1a1a1a; \
    --border-default: #222; \
    --border-strong: #333; \
    --text-primary: #d4d4d4; \
    --text-secondary: #888; \
    --text-tertiary: #525252; \
    --accent-blue: #60a5fa; \
    --accent-green: #22c55e; \
    --accent-red: #ef4444; \
    --accent-amber: #f59e0b; \
    --space-xs: 2px; \
    --space-sm: 4px; \
    --space-md: 8px; \
    --space-lg: 12px; \
    --space-xl: 16px; \
    --space-2xl: 24px; \
} \
window { background-color: var(--bg-primary); } \
.sidebar { background-color: var(--bg-secondary); border-right: 1px solid var(--border-subtle); } \
/* ... rest of CSS using variables */";
```

- [ ] **Step 3: Update all CSS rules to use variables**

Replace hardcoded values:
- `#0a0a0a` → `var(--bg-primary)`
- `#0d0d0d` → `var(--bg-secondary)`
- `#141414` → `var(--bg-tertiary)`
- `#1a1a1a` → `var(--border-subtle)`
- `#222` → `var(--border-default)`
- `#333` → `var(--border-strong)`
- `#d4d4d4` → `var(--text-primary)`
- `#888` → `var(--text-secondary)`
- `#525252` → `var(--text-tertiary)`
- `#60a5fa` → `var(--accent-blue)`
- `#22c55e` → `var(--accent-green)`
- `#ef4444` → `var(--accent-red)`
- `#f59e0b` → `var(--accent-amber)`

- [ ] **Step 4: Build and verify**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 5: Commit**

```bash
git add src/infrastructure/nvr/css.rs
git commit -m "feat: add CSS custom property tokens for design system"
```

---

### Task 2: Spacing Constants

**Covers:** [S5]

**Files:**
- Modify: `src/infrastructure/nvr/constants.rs`

- [ ] **Step 1: Add spacing constants**

Add to `constants.rs`:

```rust
/// Spacing scale (4px base)
pub(crate) const SPACE_XS: i32 = 2;
pub(crate) const SPACE_SM: i32 = 4;
pub(crate) const SPACE_MD: i32 = 8;
pub(crate) const SPACE_LG: i32 = 12;
pub(crate) const SPACE_XL: i32 = 16;
pub(crate) const SPACE_2XL: i32 = 24;

/// Border radius
pub(crate) const RADIUS_BADGE: i32 = 3;
pub(crate) const RADIUS_CARD: i32 = 6;

/// Hit target minimum size
pub(crate) const HIT_TARGET_MIN: i32 = 40;
```

- [ ] **Step 2: Build and verify**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 3: Commit**

```bash
git add src/infrastructure/nvr/constants.rs
git commit -m "feat: add spacing scale constants"
```

---

### Task 3: Sidebar Improvements

**Covers:** [S6]

**Files:**
- Modify: `src/infrastructure/nvr/css.rs`
- Modify: `src/infrastructure/nvr/sidebar.rs`

- [ ] **Step 1: Update sidebar CSS**

Add to CSS theme:

```css
.sidebar-row:hover { background-color: var(--bg-tertiary); }
.sidebar-row-selected { border-left: 3px solid var(--accent-blue); }
.status-online { color: var(--accent-green); }
.status-offline { color: var(--accent-red); }
.status-recording { color: var(--accent-amber); }
```

- [ ] **Step 2: Update sidebar.rs for selection indicator**

In `build_sidebar`, add selection visual feedback. When a row is selected, add the `sidebar-row-selected` class.

- [ ] **Step 3: Update status dot colors**

In `SidebarRow`, update the `dot` label to use semantic colors based on camera status.

- [ ] **Step 4: Build and verify**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/infrastructure/nvr/css.rs src/infrastructure/nvr/sidebar.rs
git commit -m "feat: improve sidebar with selection indicator and status colors"
```

---

### Task 4: Toolbar Improvements

**Covers:** [S6]

**Files:**
- Modify: `src/infrastructure/nvr/css.rs`
- Modify: `src/infrastructure/nvr/mod.rs`

- [ ] **Step 1: Update toolbar CSS**

Add to CSS theme:

```css
.toolbar-btn:hover { background-color: var(--bg-tertiary); border-color: var(--border-strong); }
.toolbar-btn:active { background-color: var(--border-default); }
.toolbar-btn:checked { background-color: rgba(37,99,235,0.2); color: var(--accent-blue); border-color: var(--accent-blue); }
```

- [ ] **Step 2: Update toolbar.rs for hover states**

The CSS changes should handle hover/active states. Verify the toolbar buttons respond correctly.

- [ ] **Step 3: Build and verify**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/css.rs
git commit -m "feat: improve toolbar button hover and active states"
```

---

### Task 5: Info Panel Improvements

**Covers:** [S8]

**Files:**
- Modify: `src/infrastructure/nvr/css.rs`
- Modify: `src/infrastructure/nvr/info_panel.rs`

- [ ] **Step 1: Update info panel CSS**

Add to CSS theme:

```css
.info-panel { padding: var(--space-md); }
.info-section { margin-bottom: var(--space-lg); }
.info-label { color: var(--text-secondary); font-size: 8pt; }
.info-value { color: var(--text-primary); font-size: 8pt; }
.info-separator { border-top: 1px solid var(--border-subtle); margin: var(--space-md) 0; }
```

- [ ] **Step 2: Update info_panel.rs for consistent spacing**

Update `InfoPanel::new` to use spacing constants for margins and padding.

- [ ] **Step 3: Update metric labels for alignment**

Ensure metric labels use consistent typography and alignment.

- [ ] **Step 4: Build and verify**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/infrastructure/nvr/css.rs src/infrastructure/nvr/info_panel.rs
git commit -m "feat: improve info panel with consistent spacing and typography"
```

---

### Task 6: Camera Cell Improvements

**Covers:** [S7]

**Files:**
- Modify: `src/infrastructure/nvr/css.rs`
- Modify: `src/infrastructure/nvr/cell.rs`

- [ ] **Step 1: Update cell CSS**

Add to CSS theme:

```css
.cell-frame { transition: border-color 0.15s ease; }
.cell-frame:hover { border-color: var(--border-strong); }
.cell-frame-selected { border-color: var(--accent-green); }
.cell-frame-offline { border-color: var(--accent-red); }
.cell-frame-recording { border-color: var(--accent-amber); border-style: dashed; }
.cell-loading { background: linear-gradient(90deg, var(--bg-primary) 25%, var(--bg-tertiary) 50%, var(--bg-primary) 75%); background-size: 200% 100%; animation: loading 1.5s infinite; }
@keyframes loading { 0% { background-position: 200% 0; } 100% { background-position: -200% 0; } }
```

- [ ] **Step 2: Update cell.rs for state classes**

Update `CameraCell` to apply appropriate CSS classes based on state (online/offline/recording).

- [ ] **Step 3: Build and verify**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 4: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 5: Commit**

```bash
git add src/infrastructure/nvr/css.rs src/infrastructure/nvr/cell.rs
git commit -m "feat: improve camera cells with state-based styling"
```

---

### Task 7: Toast Improvements

**Covers:** [S7]

**Files:**
- Modify: `src/infrastructure/nvr/css.rs`
- Modify: `src/infrastructure/nvr/toast.rs`

- [ ] **Step 1: Update toast CSS**

Add to CSS theme:

```css
.toast-success { background-color: rgba(34,197,94,0.9); color: #000; }
.toast-error { background-color: rgba(239,68,68,0.9); color: #fff; }
.toast-warning { background-color: rgba(245,158,11,0.9); color: #000; }
.toast-info { background-color: rgba(30,30,30,0.92); color: var(--text-primary); }
```

- [ ] **Step 2: Update toast.rs for semantic types**

Add a `ToastType` enum and update `Toast::show` to accept a type parameter.

- [ ] **Step 3: Update callers to use semantic types**

Update all `toast.show("...")` calls to use the appropriate type.

- [ ] **Step 4: Build and verify**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 5: Run tests**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 6: Commit**

```bash
git add src/infrastructure/nvr/css.rs src/infrastructure/nvr/toast.rs
git commit -m "feat: add semantic toast types for success/error/warning/info"
```

---

### Task 8: Icon Bar Improvements

**Covers:** [S6]

**Files:**
- Modify: `src/infrastructure/nvr/css.rs`
- Modify: `src/infrastructure/nvr/icon_bar.rs`

- [ ] **Step 1: Update icon bar CSS**

Add to CSS theme:

```css
.icon-btn { min-width: var(--HIT_TARGET_MIN); min-height: var(--HIT_TARGET_MIN); }
.icon-btn:hover { background-color: var(--bg-tertiary); }
.icon-btn:checked { background-color: var(--border-default); outline: 2px solid var(--accent-green); }
```

- [ ] **Step 2: Add tooltips to icon buttons**

Update `IconBar::new` to add tooltips to each button.

- [ ] **Step 3: Build and verify**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/css.rs src/infrastructure/nvr/icon_bar.rs
git commit -m "feat: improve icon bar with tooltips and hit targets"
```

---

### Task 9: Typography Consistency

**Covers:** [S4]

**Files:**
- Modify: `src/infrastructure/nvr/css.rs`

- [ ] **Step 1: Audit current font sizes**

Review all `font-size` declarations in CSS and ensure they follow the scale:
- Display: 10pt
- Heading: 9pt
- Body: 8pt
- Caption: 7pt
- Micro: 7pt

- [ ] **Step 2: Update any inconsistent sizes**

Replace non-standard sizes with the nearest scale value.

- [ ] **Step 3: Build and verify**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 4: Commit**

```bash
git add src/infrastructure/nvr/css.rs
git commit -m "feat: enforce typography scale consistency"
```

---

### Task 10: Final Verification

**Covers:** [S10]

**Files:**
- None (verification only)

- [ ] **Step 1: Full build**

Run: `cargo build`
Expected: Zero warnings

- [ ] **Step 2: Full test suite**

Run: `cargo test`
Expected: All tests pass

- [ ] **Step 3: Visual verification**

Launch the application and verify:
- [ ] Dark theme applied consistently
- [ ] Sidebar hover states work
- [ ] Camera selection indicator visible
- [ ] Info panel spacing consistent
- [ ] Toast notifications semantic colors
- [ ] Icon bar tooltips appear on hover
- [ ] All buttons have visible hover/active states

- [ ] **Step 4: Commit final state**

```bash
git add -A
git commit -m "feat: complete UI design system implementation"
```
