# UX/UI Consistency Fixes — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task.

**Goal:** Fix all UI/UX inconsistencies identified by the UX/UI audit — positioning, colors, click targets, sizes, interactions, and dead code.

**Architecture:** Pure CSS + widget property changes. No new modules needed. All fixes are in `src/infrastructure/nvr/`.

---

## File Structure

| File | Changes |
|------|---------|
| `css.rs` | Fix colors, borders, spacing, add hover states, remove dead CSS |
| `icon_bar.rs` | Fix button sizing, add hover feedback |
| `info_panel.rs` | Fix VU meter height, font sizes, spacing |
| `sidebar.rs` | Fix row heights, checkbox alignment |
| `cell.rs` | Fix HUD overlay background, record label font |
| `tick.rs` | Fix status bar contrast, VU meter height, merge status updates |
| `mod.rs` | Fix paned position, minimum window size, merge status bar |
| `keyboard.rs` | Fix F3 icon desync, space key behavior |
| `constants.rs` | Add new constants, fix values |
| `layout.rs` | Remove dead `update_status` function |

---

## Task 1: Fix critical interaction bugs

**Priority: CRITICAL**

- [ ] **Fix F3 keyboard icon bar desync** (`keyboard.rs`)
  - F3 currently toggles visibility without updating icon bar buttons
  - Add icon bar reference to KeyboardContext
  - Update icon bar state when F3 is pressed
  - Cycle through all 3 panels (cameras → info → diagnostics → cameras)

- [ ] **Fix cell deselection on re-click** (`mod.rs`)
  - Currently clicking a selected cell does nothing
  - Add: if `selected_idx.get() == Some(i)`, deselect and show camera list
  - Update icon bar to show Cameras panel

- [ ] **Fix toast timeout stacking** (`toast.rs`)
  - Store `SourceId` from `timeout_add_local_once`
  - Cancel previous timeout before setting new one
  - Use `Rc<Cell<Option<glib::SourceId>>>`

- [ ] **Fix space key behavior** (`keyboard.rs`)
  - Space should toggle pause/resume (matching AGENTS.md)
  - Or update help popup to say "Select camera" instead of "Pause/resume"
  - Check what single-camera mode does and match

---

## Task 2: Fix critical visual issues

**Priority: CRITICAL**

- [ ] **Add HUD overlay background** (`cell.rs` + `css.rs`)
  - `hud_lbl` has no background — invisible on bright video
  - Add CSS class `.cell-hud` with `background-color: rgba(0,0,0,0.45)`
  - Add `padding: 1px 4px; border-radius: 2px`

- [ ] **Fix status bar contrast** (`css.rs`)
  - Current: `#4a4a4a` on `#050505` = ~2.3:1 contrast (fails WCAG)
  - Change to `#6b7280` or `#888` for readable text

- [ ] **Fix cell border jitter** (`css.rs`)
  - Current: selected state uses `border: 3px solid #22c55e` (jumps from 1px)
  - Fix: Use `outline: 2px solid #22c55e` instead of changing border width
  - Or: use consistent 2px border for all states

- [ ] **Merge duplicate status bar updates** (`tick.rs` + `layout.rs`)
  - `tick.rs:update_status_bar` runs every 250ms, overwrites `layout.rs:update_status`
  - Remove `update_status` from `layout.rs`
  - Integrate layout mode info into `update_status_bar`

---

## Task 3: Fix sizing issues

**Priority: IMPORTANT**

- [ ] **Fix icon bar button overflow** (`icon_bar.rs` + `css.rs`)
  - Container: 40px, buttons: 36px + 8px padding = overflow
  - Fix: Increase container to 48px OR reduce button min-width to 32px
  - Recommended: container 48px, button 40px

- [ ] **Fix paned position mismatch** (`mod.rs` + `constants.rs`)
  - Current: `paned.set_position(320)` but SIDEBAR_WIDTH=260 + ICON_BAR_WIDTH=40 = 300
  - Fix: Use `constants::SIDEBAR_WIDTH + constants::ICON_BAR_WIDTH`

- [ ] **Add minimum window size** (`mod.rs`)
  - Currently no minimum — window can shrink to unusable size
  - Add: `window.set_size_request(800, 480)`

- [ ] **Increase VU meter height** (`info_panel.rs`)
  - Current: 16px — too small for 24 LED segments
  - Fix: Increase to 24px

- [ ] **Fix sidebar row minimum height** (`sidebar.rs`)
  - Current: ~28-32px depending on font rendering
  - Fix: Add `row.set_size_request(-1, 36)` for consistent height

---

## Task 4: Standardize colors and fonts

**Priority: IMPORTANT**

- [ ] **Consolidate gray palette** (`css.rs`)
  - Current: 6 distinct grays (#4a4a4a, #525252, #6b7280, #888, #a3a3a3, #333)
  - Fix: Reduce to 3: `#888` (medium), `#6b7280` (muted), `#525252` (dimmest)
  - Update all references

- [ ] **Consolidate border colors** (`css.rs`)
  - Current: 6 distinct border colors
  - Fix: Reduce to 3: `#1a1a1a` (subtle), `#333` (medium), `#222` (cells)

- [ ] **Standardize font sizes** (`info_panel.rs` + `css.rs`)
  - Current: 7pt, 8pt, 9pt, 10pt, 11pt for various purposes
  - Fix: Use 3 sizes: 7pt (micro/labels), 9pt (body), 11pt (heading)
  - Update info panel section titles to consistent 8pt

- [ ] **Fix sparkline green color** (`info_panel.rs`)
  - Current: `rgba(0.13, 0.67, 0.13)` ≈ #21ab21
  - Fix: Use `rgba(0.133, 0.773, 0.369)` to match #22c55e accent

- [ ] **Fix panel background consistency** (`css.rs`)
  - Current: `.panel-box` uses `#111`, `.sidebar` uses `#0d0d0d`
  - Fix: Standardize to `#0d0d0d`

---

## Task 5: Remove dead code and polish

**Priority: NICE TO HAVE**

- [ ] **Remove dead CSS classes** (`css.rs`)
  - Remove: `.info-frame`, `.camera-label`, `.stats-text`, `.offline-msg`
  - Verify no code references them

- [ ] **Add hover states** (`css.rs`)
  - Add: `.icon-btn:hover { background-color: #2a2a2a; }`
  - Add: `.toolbar-btn:hover { background-color: #222; border-color: #3a3a3a; }`

- [ ] **Fix record label font** (`tick.rs`)
  - Current: no font specified in markup
  - Fix: Add `font='monospace 7'` to record label markup

- [ ] **Fix icon bar emoji font size** (`icon_bar.rs`)
  - Current: no explicit font-size for emoji
  - Fix: Set `<span font='monospace 14'>` on labels

- [ ] **Fix toast border color** (`css.rs`)
  - Current: `#444` (outlier)
  - Fix: Use `#333` for consistency

- [ ] **Fix VU meter zone color discontinuity** (`info_panel.rs`)
  - Green zone formula creates brightness jump at boundary
  - Smooth the green channel across zone boundaries

- [ ] **Remove dead `update_status` function** (`layout.rs`)
  - This function is called but immediately overwritten by tick
  - Either remove or integrate into `update_status_bar`

---

## Verification

After all tasks:
```bash
cargo build 2>&1 | grep -E "error|warning"
cargo test 2>&1 | grep "test result"
cargo run 2>&1 &
sleep 5 && kill %1
```
Expected: Zero errors, 331 tests pass, app starts correctly.
