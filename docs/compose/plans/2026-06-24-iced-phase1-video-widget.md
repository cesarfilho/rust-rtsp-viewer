# Iced Migration — Phase 1: GStreamer Video Widget

> **For agentic workers:** REQUIRED SUB-SKILL: Use compose:subagent (recommended) or compose:execute to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create an Iced widget that renders GStreamer video frames, replacing the GTK3 gtksink approach.

**Architecture:** GStreamer pipeline with `appsink` emits RGBA frames → shared buffer → Iced custom widget polls latest frame → renders as `iced::widget::Image`. Preserves all existing domain logic and GStreamer pipeline construction.

**Tech Stack:** Rust, Iced 0.13, GStreamer 0.20, gstreamer-app 0.20

## Global Constraints

- Edition 2024 (as per Cargo.toml)
- GStreamer 0.20 + gstreamer-app 0.20 (existing dependencies)
- Iced 0.13 (latest stable)
- Domain layer (`src/domain/`) must remain UNCHANGED
- All 331 existing tests must continue passing
- No GTK3 dependencies in new Iced code (GTK3 stays for single-camera mode during migration)

---

## File Structure

| File | Responsibility |
|------|---------------|
| `src/iced_app/mod.rs` | Iced application entry point, `Application` impl |
| `src/iced_app/video_widget.rs` | Custom Iced widget for GStreamer video rendering |
| `src/iced_app/gstreamer_bridge.rs` | Bridge: GStreamer appsink → shared RGBA buffer |
| `src/iced_app/theme.rs` | Theme system (Dark/Light/AMOLED/Custom) |
| `src/iced_app/video.rs` | Video state management (pipeline lifecycle) |
| `src/bin/iced_viewer.rs` | New binary entry point for Iced mode |

---

### Task 1: Add Iced dependency and create module skeleton

**Covers:** [S3]

**Files:**
- Modify: `Cargo.toml`
- Create: `src/iced_app/mod.rs`
- Create: `src/iced_app/video_widget.rs`
- Create: `src/iced_app/gstreamer_bridge.rs`

**Interfaces:**
- Consumes: existing `gstreamer` and `gstreamer-app` dependencies
- Produces: `GStreamerBridge` struct with `latest_frame()` method

- [ ] **Step 1: Add Iced dependency to Cargo.toml**

Add after line 26:
```toml
iced = { version = "0.13", features = ["image", "tokio"] }
```

- [ ] **Step 2: Create module skeleton**

Create `src/iced_app/mod.rs`:
```rust
pub mod video_widget;
pub mod gstreamer_bridge;
pub mod video;
pub mod theme;
```

Create `src/iced_app/video_widget.rs` (empty for now):
```rust
// Placeholder — implemented in Task 3
```

Create `src/iced_app/gstreamer_bridge.rs`:
```rust
// Placeholder — implemented in Task 2
```

- [ ] **Step 3: Verify build**

Run: `cargo check 2>&1 | head -20`
Expected: Compiles with Iced dependency resolved, no errors

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml src/iced_app/
git commit -m "chore: add iced dependency and module skeleton"
```

---

### Task 2: GStreamer Bridge (appsink → RGBA buffer)

**Covers:** [S3]

**Files:**
- Create: `src/iced_app/gstreamer_bridge.rs`
- Test: unit test in same file

**Interfaces:**
- Consumes: `gstreamer` 0.20, `gstreamer-app` 0.20
- Produces: `GStreamerBridge` struct, `GStreamerBridge::new(url, width, height)`, `latest_frame() -> Option<Vec<u8>>`, `frame_size() -> (u32, u32)`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_creation_returns_none_frame() {
        let bridge = GStreamerBridge::new(640, 480);
        assert!(bridge.latest_frame().is_none());
    }

    #[test]
    fn bridge_default_size() {
        let bridge = GStreamerBridge::new(1920, 1080);
        assert_eq!(bridge.frame_size(), (1920, 1080));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test iced_app::gstreamer_bridge::tests`
Expected: FAIL — module not found

- [ ] **Step 3: Implement GStreamerBridge**

```rust
use std::sync::{Arc, Mutex};
use gstreamer as gst;
use gst::prelude::*;
use gstreamer_app::{self, appsink::AppSinkExt, prelude::*};

pub struct GStreamerBridge {
    pipeline: Option<gst::Pipeline>,
    frame_buffer: Arc<Mutex<Option<Vec<u8>>>>,
    frame_size: Arc<Mutex<(u32, u32)>>,
    width: u32,
    height: u32,
}

impl GStreamerBridge {
    pub fn new(width: u32, height: u32) -> Self {
        let _ = gst::init();
        Self {
            pipeline: None,
            frame_buffer: Arc::new(Mutex::new(None)),
            frame_size: Arc::new(Mutex::new((width, height))),
            width,
            height,
        }
    }

    pub fn start_rtsp(&mut self, url: &str, latency_ms: u32) -> Result<(), String> {
        let pipeline_str = format!(
            "rtspsrc location={} latency={} buffer-mode=auto \
             ! decodebin ! videoconvert \
             ! video/x-raw,format=RGBA,width={},height={} \
             ! appsink name=sink emit-signals=true sync=false",
            url, latency_ms, self.width, self.height
        );

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Pipeline parse error: {}", e))?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| "Not a pipeline".to_string())?;

        let appsink = pipeline.by_name("sink")
            .ok_or("No appsink element")?
            .dynamic_cast::<gstreamer_app::AppSink>()
            .map_err(|_| "Not an appsink".to_string())?;

        let buffer = self.frame_buffer.clone();
        let size = self.frame_size.clone();
        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample()
                        .map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer()
                        .ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable()
                        .map_err(|_| gst::FlowError::Error)?;
                    let caps = sample.caps()
                        .ok_or(gst::FlowError::Error)?;
                    let structure = caps.structure(0)
                        .ok_or(gst::FlowError::Error)?;

                    let w = structure.get::<i32>("width").unwrap_or(0) as u32;
                    let h = structure.get::<i32>("height").unwrap_or(0) as u32;

                    let mut frame_buf = buffer.lock().unwrap();
                    *frame_buf = Some(map.to_vec());
                    drop(frame_buf);

                    let mut sz = size.lock().unwrap();
                    *sz = (w, h);

                    Ok(gst::FlowSuccess::Ok)
                })
                .build()
        );

        pipeline.set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to set state: {:?}", e))?;

        self.pipeline = Some(pipeline);
        Ok(())
    }

    pub fn start_file(&mut self, path: &str) -> Result<(), String> {
        let pipeline_str = format!(
            "filesrc location={} ! decodebin ! videoconvert \
             ! video/x-raw,format=RGBA,width={},height={} \
             ! appsink name=sink emit-signals=true sync=false",
            path, self.width, self.height
        );

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Pipeline parse error: {}", e))?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| "Not a pipeline".to_string())?;

        let appsink = pipeline.by_name("sink")
            .ok_or("No appsink element")?
            .dynamic_cast::<gstreamer_app::AppSink>()
            .map_err(|_| "Not an appsink".to_string())?;

        let buffer = self.frame_buffer.clone();
        let size = self.frame_size.clone();
        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample()
                        .map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer()
                        .ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable()
                        .map_err(|_| gst::FlowError::Error)?;
                    let caps = sample.caps()
                        .ok_or(gst::FlowError::Error)?;
                    let structure = caps.structure(0)
                        .ok_or(gst::FlowError::Error)?;

                    let w = structure.get::<i32>("width").unwrap_or(0) as u32;
                    let h = structure.get::<i32>("height").unwrap_or(0) as u32;

                    let mut frame_buf = buffer.lock().unwrap();
                    *frame_buf = Some(map.to_vec());
                    drop(frame_buf);

                    let mut sz = size.lock().unwrap();
                    *sz = (w, h);

                    Ok(gst::FlowSuccess::Ok)
                })
                .build()
        );

        pipeline.set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to set state: {:?}", e))?;

        self.pipeline = Some(pipeline);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(pipeline) = self.pipeline.take() {
            let _ = pipeline.set_state(gst::State::Null);
        }
    }

    pub fn latest_frame(&self) -> Option<Vec<u8>> {
        self.frame_buffer.lock().unwrap().clone()
    }

    pub fn frame_size(&self) -> (u32, u32) {
        *self.frame_size.lock().unwrap()
    }
}

impl Drop for GStreamerBridge {
    fn drop(&mut self) {
        self.stop();
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test iced_app::gstreamer_bridge::tests`
Expected: PASS (2 tests)

- [ ] **Step 5: Commit**

```bash
git add src/iced_app/gstreamer_bridge.rs
git commit -m "feat: add GStreamer bridge for Iced video rendering"
```

---

### Task 3: Iced Video Widget

**Covers:** [S3]

**Files:**
- Create: `src/iced_app/video_widget.rs`
- Test: unit test in same file

**Interfaces:**
- Consumes: `GStreamerBridge::latest_frame()`, `GStreamerBridge::frame_size()`
- Produces: `VideoWidget` Iced widget with `view()` method returning `Element`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_widget_creation() {
        let bridge = std::sync::Arc::new(std::sync::Mutex::new(
            crate::iced_app::gstreamer_bridge::GStreamerBridge::new(640, 480)
        ));
        let widget = VideoWidget::new(bridge);
        // Widget should be constructible without panic
        let _ = widget;
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test iced_app::video_widget::tests`
Expected: FAIL — module not found

- [ ] **Step 3: Implement VideoWidget**

```rust
use std::sync::{Arc, Mutex};
use iced::{Element, Length, widget::image::Handle};
use crate::iced_app::gstreamer_bridge::GStreamerBridge;

#[derive(Clone)]
pub struct VideoWidget {
    bridge: Arc<Mutex<GStreamerBridge>>,
    width: u32,
    height: u32,
}

impl VideoWidget {
    pub fn new(bridge: Arc<Mutex<GStreamerBridge>>) -> Self {
        let (w, h) = bridge.lock().unwrap().frame_size();
        Self { bridge, width: w, height: h }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let frame = self.bridge.lock().unwrap().latest_frame();
        let handle = match frame {
            Some(data) => {
                let (w, h) = self.bridge.lock().unwrap().frame_size();
                Handle::from_pixels(w, h, iced::image::Bytes::Owned(data))
            }
            None => {
                Handle::from_pixels(self.width, self.height, vec![20u8; (self.width * self.height * 4) as usize])
            }
        };
        iced::widget::image(handle)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    FrameUpdate,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test iced_app::video_widget::tests`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/iced_app/video_widget.rs
git commit -m "feat: add Iced video widget for GStreamer rendering"
```

---

### Task 4: Iced Application Shell

**Covers:** [S3, S4]

**Files:**
- Create: `src/iced_app/mod.rs` (rewrite)
- Create: `src/bin/iced_viewer.rs`
- Modify: `Cargo.toml` (add bin target)

**Interfaces:**
- Consumes: `VideoWidget`, `GStreamerBridge`
- Produces: runnable Iced application with video display

- [ ] **Step 1: Rewrite iced_app/mod.rs with Application**

```rust
pub mod video_widget;
pub mod gstreamer_bridge;
pub mod video;
pub mod theme;

use iced::{Element, Length, Settings, Task};
use std::sync::{Arc, Mutex};
use gstreamer as gst;
use gst::prelude::*;

use video_widget::VideoWidget;
use gstreamer_bridge::GStreamerBridge;

pub fn run(url: String) -> iced::Result {
    let settings = Settings {
        window: iced::window::Settings {
            size: iced::Size::new(1280.0, 720.0),
            ..Default::default()
        },
        ..Default::default()
    };
    iced::application("rust-rtsp-viewer", App::update, App::view)
        .settings(settings)
        .run_with(App::new(url))
}

pub struct App {
    bridge: Arc<Mutex<GStreamerBridge>>,
    video: VideoWidget,
    url: String,
}

impl App {
    fn new(url: String) -> (Self, Task<Message>) {
        let _ = gst::init();
        let mut bridge = GStreamerBridge::new(1920, 1080);
        let _ = bridge.start_rtsp(&url, 100);
        let bridge = Arc::new(Mutex::new(bridge));
        let video = VideoWidget::new(bridge.clone());

        (Self { bridge, video, url }, Task::none())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FrameUpdate => {}
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        self.video.view()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    FrameUpdate,
}
```

- [ ] **Step 2: Create bin entry point**

Create `src/bin/iced_viewer.rs`:
```rust
fn main() {
    env_logger::init();
    let url = std::env::args().nth(1).expect("Usage: iced_viewer <URL>");
    rust_rtsp_viewer::iced_app::run(url).unwrap();
}
```

- [ ] **Step 3: Add bin target to Cargo.toml**

Add after `[dependencies]`:
```toml
[[bin]]
name = "iced_viewer"
path = "src/bin/iced_viewer.rs"
```

- [ ] **Step 4: Verify build**

Run: `cargo check --bin iced_viewer 2>&1 | head -30`
Expected: Compiles without errors

- [ ] **Step 5: Commit**

```bash
git add src/iced_app/mod.rs src/bin/iced_viewer.rs Cargo.toml
git commit -m "feat: add Iced application shell with video display"
```

---

### Task 5: Theme System

**Covers:** [S4]

**Files:**
- Create: `src/iced_app/theme.rs`
- Test: unit test in same file

**Interfaces:**
- Consumes: none (standalone)
- Produces: `Theme` enum, `theme_colors()` → `ThemeColors` struct

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_colors() {
        let colors = Theme::Dark.colors();
        assert_eq!(colors.background, "#0a0a0a");
        assert_eq!(colors.text, "#d4d4d4");
    }

    #[test]
    fn light_theme_colors() {
        let colors = Theme::Light.colors();
        assert_eq!(colors.background, "#ffffff");
        assert_eq!(colors.text, "#1a1a1a");
    }

    #[test]
    fn amoled_theme_colors() {
        let colors = Theme::Amoled.colors();
        assert_eq!(colors.background, "#000000");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test iced_app::theme::tests`
Expected: FAIL — module not found

- [ ] **Step 3: Implement theme.rs**

```rust
use iced::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
    Amoled,
    Custom,
}

pub struct ThemeColors {
    pub background: &'static str,
    pub surface: &'static str,
    pub surface_hover: &'static str,
    pub border: &'static str,
    pub border_strong: &'static str,
    pub text: &'static str,
    pub text_secondary: &'static str,
    pub text_tertiary: &'static str,
    pub accent_blue: &'static str,
    pub accent_green: &'static str,
    pub accent_red: &'static str,
    pub accent_amber: &'static str,
}

impl Theme {
    pub fn colors(&self) -> ThemeColors {
        match self {
            Theme::Dark => ThemeColors {
                background: "#0a0a0a",
                surface: "#0d0d0d",
                surface_hover: "#141414",
                border: "#222222",
                border_strong: "#333333",
                text: "#d4d4d4",
                text_secondary: "#888888",
                text_tertiary: "#525252",
                accent_blue: "#60a5fa",
                accent_green: "#22c55e",
                accent_red: "#ef4444",
                accent_amber: "#f59e0b",
            },
            Theme::Light => ThemeColors {
                background: "#ffffff",
                surface: "#f5f5f5",
                surface_hover: "#e8e8e8",
                border: "#dddddd",
                border_strong: "#bbbbbb",
                text: "#1a1a1a",
                text_secondary: "#555555",
                text_tertiary: "#999999",
                accent_blue: "#2563eb",
                accent_green: "#16a34a",
                accent_red: "#dc2626",
                accent_amber: "#d97706",
            },
            Theme::Amoled => ThemeColors {
                background: "#000000",
                surface: "#000000",
                surface_hover: "#111111",
                border: "#111111",
                border_strong: "#222222",
                text: "#d4d4d4",
                text_secondary: "#888888",
                text_tertiary: "#525252",
                accent_blue: "#60a5fa",
                accent_green: "#22c55e",
                accent_red: "#ef4444",
                accent_amber: "#f59e0b",
            },
            Theme::Custom => ThemeColors {
                background: "#1a1a2e",
                surface: "#16213e",
                surface_hover: "#0f3460",
                border: "#533483",
                border_strong: "#7b2d8e",
                text: "#eaeaea",
                text_secondary: "#b8b8b8",
                text_tertiary: "#6c6c6c",
                accent_blue: "#60a5fa",
                accent_green: "#22c55e",
                accent_red: "#ef4444",
                accent_amber: "#f59e0b",
            },
        }
    }

    pub fn color_from_hex(hex: &str) -> Color {
        let hex = hex.trim_start_matches('#');
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
        Color::from_rgb(r, g, b)
    }

    pub fn all() -> &'static [Theme] {
        &[Theme::Dark, Theme::Light, Theme::Amoled, Theme::Custom]
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test iced_app::theme::tests`
Expected: PASS (3 tests)

- [ ] **Step 5: Commit**

```bash
git add src/iced_app/theme.rs
git commit -m "feat: add multi-theme system (Dark/Light/AMOLED/Custom)"
```

---

### Task 6: Integration test — run Iced viewer

**Covers:** [S3, S4]

**Files:**
- No new files — verification only

**Interfaces:**
- Consumes: all previous tasks
- Produces: visual confirmation of video rendering

- [ ] **Step 1: Build the iced_viewer binary**

Run: `cargo build --bin iced_viewer 2>&1 | tail -5`
Expected: `Finished dev profile [unoptimized + debuginfo]`

- [ ] **Step 2: Run with a test stream**

Run: `cargo run --bin iced_viewer -- rtsp://localhost:8554/test 2>&1 | head -20`
Expected: Window opens with video rendering (or connection error if no test stream)

- [ ] **Step 3: Run all existing tests**

Run: `cargo test 2>&1 | grep "test result"`
Expected: All 331 tests pass

- [ ] **Step 4: Commit (if any fixes needed)**

```bash
git add -A
git commit -m "fix: integration fixes for Iced viewer"
```
