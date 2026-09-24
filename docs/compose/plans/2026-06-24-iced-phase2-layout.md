# Phase 2: Layout + Theme Integration

> **For agentic workers:** Use compose:subagent or compose:execute to implement task-by-task.

**Goal:** Add toolbar, sidebar panel, and display area to the Iced application with theme selector.

**Architecture:** Iced functional UI with `row`/`column` layout containers. Theme state stored in `App`, applied to all widgets via `ThemeColors`.

**Tech Stack:** Iced 0.13, existing GStreamer bridge

## Tasks

### Task 1: Theme selector in toolbar

**Files:**
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Add `theme: Theme` field to `App`
2. Add `Message::ThemeChanged(Theme)` variant
3. Build toolbar with theme dropdown using `iced::widget::pick_list`
4. Wire theme change to update `app.theme`

### Task 2: Sidebar panel structure

**Files:**
- Create: `src/iced_app/sidebar.rs`
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Create `Sidebar` struct with camera list placeholder
2. Add `Message::SidebarViewChanged(SidebarView)` for tab switching
3. Wire sidebar into main layout with `row(container(sidebar), container(video))`

### Task 3: Layout styling with theme

**Files:**
- Modify: `src/iced_app/mod.rs`
- Modify: `src/iced_app/sidebar.rs`

**Steps:**
1. Apply theme colors to all containers (background, borders)
2. Style toolbar, sidebar, and display area
3. Verify theme switching works

### Task 4: Integration test

**Steps:**
1. Build and verify
2. Run all tests
3. Commit
