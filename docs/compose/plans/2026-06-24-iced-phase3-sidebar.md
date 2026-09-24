# Phase 3: Sidebar — Camera List + Info Panel

> **For agentic workers:** Use compose:subagent or compose:execute to implement task-by-task.

**Goal:** Implement full sidebar functionality — camera list, search, info panel with metrics.

**Architecture:** Sidebar state holds camera data, search query, and selected camera. View renders based on active tab.

**Tech Stack:** Iced 0.13, existing theme system

## Tasks

### Task 1: Camera list with status dots

**Files:**
- Modify: `src/iced_app/sidebar.rs`

**Steps:**
1. Add `cameras: Vec<CameraInfo>` to `Sidebar`
2. Create `CameraInfo` struct (name, status, fps, bitrate, latency)
3. Render camera rows with status dot + name + compact stats
4. Add `Message::CameraClicked(usize)` handler

### Task 2: Search/filter entry

**Files:**
- Modify: `src/iced_app/sidebar.rs`

**Steps:**
1. Add `search_query: String` to `Sidebar`
2. Add `Message::SearchChanged(String)` handler
3. Render `text_input` above camera list
4. Filter cameras by search query

### Task 3: Info panel with metrics

**Files:**
- Modify: `src/iced_app/sidebar.rs`

**Steps:**
1. Add `selected_camera: Option<usize>` to `Sidebar`
2. Render info panel with metrics grid when camera selected
3. Show FPS, latency, jitter, bitrate, codec, uptime, health
4. Auto-switch to Info tab when camera clicked

### Task 4: Integration test

**Steps:**
1. Build and verify
2. Run all tests
3. Commit
