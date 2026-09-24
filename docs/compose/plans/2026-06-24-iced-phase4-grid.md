# Phase 4: Grid Auto-Layout + Multi-Camera Display

> **For agentic workers:** Use compose:subagent or compose:execute to implement task-by-task.

**Goal:** Support multiple cameras in a responsive grid that adapts to window size.

**Architecture:** Grid calculation considers camera count + window dimensions. Multiple GStreamerBridge instances feed multiple VideoWidgets. Grid/Flex mode toggle.

**Tech Stack:** Iced 0.13, existing GStreamer bridge + video widget

## Tasks

### Task 1: Grid calculation module

**Files:**
- Create: `src/iced_app/grid.rs`

**Steps:**
1. Create `calc_grid(camera_count, window_width, min_cell_size)` function
2. Return `(cols, rows, cell_width, cell_height)`
3. Add unit tests

### Task 2: Multi-camera bridge support

**Files:**
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Change `bridge: Arc<Mutex<GStreamerBridge>>` to `bridges: Vec<Arc<Mutex<GStreamerBridge>>>`
2. Change `video: VideoWidget` to `videos: Vec<VideoWidget>`
3. Initialize multiple bridges from sidebar camera list
4. Update `view()` to render grid of video widgets

### Task 3: Grid/Flex mode toggle

**Files:**
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Add `LayoutMode` enum (Grid, Flex)
2. Add `layout_mode: LayoutMode` to `App`
3. Add `Message::LayoutModeChanged(LayoutMode)` handler
4. Add toggle buttons in toolbar
5. Grid mode: render cameras in grid
6. Flex mode: render one main + thumbnails

### Task 4: Window resize handling

**Files:**
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Track window size in `App`
2. Add `Message::WindowResized(iced::Size)` handler
3. Recalculate grid on resize
4. Use `iced::window::on_resize` subscription

### Task 5: Integration test

**Steps:**
1. Build and verify
2. Run all tests
3. Commit
