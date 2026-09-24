# Complete Iced Migration — Config + Pipeline Integration

> **For agentic workers:** Use compose:subagent or compose:execute to implement task-by-task.

**Goal:** Make Iced viewer read config.toml, use existing GStreamer pipelines, and become the default binary.

**Architecture:** Reuse existing `config.rs`, `pipeline.rs`, and GStreamer pipeline construction. Replace GTK3 UI with Iced.

**Tech Stack:** Iced 0.13, existing config/pipeline/GStreamer infrastructure

## Tasks

### Task 1: Config loading for Iced viewer

**Files:**
- Create: `src/iced_app/config.rs`
- Modify: `src/iced_app/mod.rs`

**Steps:**
1. Create config adapter that reads `config.toml` using existing `Config` struct
2. Convert `CameraConfig` to Iced camera info
3. Initialize cameras from config in `new_app()`

### Task 2: Pipeline construction for Iced

**Files:**
- Modify: `src/iced_app/gstreamer_bridge.rs`

**Steps:**
1. Use existing `pipeline::build_pipeline_string()` to build GStreamer pipelines
2. Create pipelines from `CameraConfig` instead of hardcoded URLs
3. Support both RTSP and HLS pipelines

### Task 3: Camera list from config

**Files:**
- Modify: `src/iced_app/sidebar.rs`

**Steps:**
1. Populate camera list from config.toml cameras
2. Show camera names and labels
3. Initialize bridges for each camera

### Task 4: Make Iced viewer the default

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`

**Steps:**
1. Change default binary to Iced viewer
2. Keep GTK3 as fallback (`--gtk` flag)
3. Pass CLI args to Iced viewer

### Task 5: Integration test

**Steps:**
1. Build and verify
2. Run with config.toml
3. Run all tests
4. Commit
