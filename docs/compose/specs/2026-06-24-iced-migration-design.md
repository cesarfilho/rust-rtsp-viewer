# Iced Migration Design Spec

## [S1] Problem
The NVR application currently uses GTK3 for all UI. The user wants to migrate to Iced (COSMIC toolkit) for better integration with Pop!_OS COSMIC desktop, while adding multi-theme support, improved sidebar, auto-adaptive grid, and audio visual indicators.

## [S2] Solution Overview
Phased migration from GTK3 to Iced, preserving domain logic (metrics, diagnostics, GStreamer pipelines) while rewriting the UI layer. 5 phases, each independently testable.

## [S3] Phase 1 — GStreamer Video Widget
- Create `GStreamerSurface` Iced widget
- Bridge: GStreamer `appsink` (RGBA) → `iced::widget::image::Handle` → render
- Support multiple simultaneous video streams
- Test: display 1+ camera feeds in a basic Iced window

## [S4] Phase 2 — Layout Base + Multi-Theme
- Iced window with toolbar, sidebar panel, display area
- Theme system: `Theme` enum with Dark/Light/AMOLED/Custom variants
- CSS-like variable system via Iced `theme::Palette`
- Theme selector in toolbar (dropdown)
- Custom theme: user edits `themes/custom.css` or config

### Theme Definitions
| Theme | Background | Text | Border | Accent |
|-------|-----------|------|--------|--------|
| Dark | #0a0a0a | #d4d4d4 | #222 | #60a5fa |
| Light | #ffffff | #1a1a1a | #ddd | #2563eb |
| AMOLED | #000000 | #d4d4d4 | #111 | #60a5fa |
| Custom | user-defined | user-defined | user-defined | user-defined |

## [S5] Phase 3 — Sidebar
- Tab bar with GTK symbolic icons (camera-web-symbolic, dialog-information-symbolic, medical-symbolic)
- Camera list with status dots, compact stats
- Auto-switch to Info tab when camera clicked
- Search/filter entry
- Info panel with metrics grid, sparkline, VU meter

## [S6] Phase 4 — Grid Auto-Layout
- `calc_grid()` considers both camera count AND window dimensions
- Minimum cell size: 200px
- Re-calculate on `size-allocate` / window resize
- Grid mode + Flex mode toggle

### Grid Calculation Logic
```
cols = max(1, min(camera_count, floor(window_width / min_cell_size)))
rows = ceil(camera_count / cols)
```

## [S7] Phase 5 — Features
- Quick actions bar: snapshot, record, audio buttons
- Audio indicator: green glow (`box-shadow`) + ♪ badge on active audio cells
- Keyboard shortcuts (same mapping as current)
- Status bar with compact format
- Toast notifications

## [S8] Architecture Boundaries
- **Domain layer** (`src/domain/`): UNCHANGED — pure Rust, no UI dependencies
- **Infrastructure layer**: GStreamer pipelines UNCHANGED, UI widgets REWRITTEN
- **NVR module**: REWRITTEN from GTK3 to Iced
- **Main entry**: Adapted to initialize Iced instead of GTK

## [S9] Testing Strategy
- Each phase produces a runnable binary for visual verification
- Domain tests (331 existing) remain untouched
- New widget tests for Iced components
- Visual regression: screenshot comparison between phases

## [S10] Risk Mitigation
- **Video latency**: Use `appsink` with `emit-signals=true`, buffer in `Arc<Mutex<VecDeque>>`, render latest frame only
- **Multi-stream performance**: Limit to 16 concurrent decoders, use `queue` elements in GStreamer
- **Theme persistence**: Store selected theme in `config.toml`
