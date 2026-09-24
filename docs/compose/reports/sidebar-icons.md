---
feature: sidebar-icons
status: delivered
specs: []
plans:
  - docs/compose/plans/2026-06-23-sidebar-icons.md
branch: main
commits: (see git log)
---

# Sidebar Colapsável com Ícones — Final Report

## What Was Built

Replaced the 3-column NVR layout (left sidebar | center display | right info panel) with a 2-column layout featuring a collapsible icon-based navigation system. A vertical icon bar (~40px) sits at the left edge with 3 toggle buttons (📷 Cameras, 📊 Info, 🩺 Diagnostics). Clicking an icon expands the adjacent panel to show the corresponding content; clicking the active icon or a different icon switches panels. Only one panel is visible at a time.

The right info panel was removed entirely. All its content (metrics, sparkline, VU meter, diagnostics) was merged into the left panel area, accessible via the icon bar navigation.

## Architecture

### Layout Structure

```
┌──────────────────────────────────────────────────────┐
│ Toolbar (Grade | Flex | N câmeras)                   │
├──────┬───────────────────────────────────────────────┤
│ Icon │ Panel (280px)          │ Display (expandable) │
│ Bar  │ ├─ 📷 Camera list     │ ─ Camera cells       │
│ 40px │ ├─ 📊 Metrics+VU      │ ─ Video grid/flex    │
│      │ └─ 🩺 Diagnostics     │                      │
├──────┴───────────────────────────────────────────────┤
│ Seek Bar                                             │
├──────────────────────────────────────────────────────┤
│ Status Bar                                           │
└──────────────────────────────────────────────────────┘
```

### Components

| Component | File | Responsibility |
|-----------|------|----------------|
| `IconBar` | `icon_bar.rs` | 3 toggle buttons, panel state |
| `InfoPanel` | `info_panel.rs` | Metrics, sparkline, VU meter |
| `SidebarRow` | `sidebar.rs` | Camera list items |
| `TickContext` | `tick.rs` | 250ms UI updates |
| `KeyboardContext` | `keyboard.rs` | Key bindings |

### Data Flow

- `selected_idx: Rc<Cell<Option<usize>>>` — shared camera selection
- `audio_level_state: Arc<AudioLevelState>` — VU meter RMS dB values
- `vu_state: Rc<RefCell<VuState>>` — peak hold + LED rendering state
- `active_panel: Rc<Cell<Option<Panel>>>` — which panel is visible

## Usage

### Navigation

| Icon | Panel | Content |
|------|-------|---------|
| 📷 | Cameras | Camera list with status, search, checkboxes |
| 📊 | Info | Selected camera metrics, sparkline, VU meter |
| 🩺 | Diagnostics | Diagnostic hints for selected camera |

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `1-9` | Select camera N |
| `m` | Toggle mute |
| `← / →` | Adjust volume ±10% |
| `F2` | Toggle panel visibility |
| `Tab` | Next camera |
| `?` | Show shortcuts |

## Verification

- **331 tests pass** (all green)
- **Zero compiler warnings**
- **App starts successfully** with both RTSP and HLS cameras
- **Icon bar toggles correctly** between panels
- **VU meter shows real audio levels** with LED segments + dB display
- **Offline detection works** — cameras show LIVE/OFFLINE status
- **Reconnect with backoff** — exponential backoff on connection loss

## Journey Log

- [pivot] User chose icon-based navigation over horizontal tabs — more compact for sidebar
- [pivot] VU meter evolved from `gtk::LevelBar` to custom LED drawing area with peak hold
- [lesson] `CameraSlot` needed bus watch installation after `Box::leak` to avoid use-after-move
- [lesson] `video/x-raw` must be excluded from codec detection (it's a decoded format, not a codec)
- [lesson] HLS codec detection requires scanning pipeline elements at runtime (uridecodebin creates children lazily)

## Source Materials

| File | Role | Notes |
|------|------|-------|
| `docs/compose/plans/2026-06-23-sidebar-icons.md` | Implementation plan | 8 tasks, all completed |
