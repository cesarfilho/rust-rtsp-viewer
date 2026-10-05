use iced::Task;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use gstreamer::prelude::*;

use crate::domain::audio::AudioState;
use crate::domain::multi_stream::{
    MultiStreamConfig, StreamQuality, desired_quality, stream_url_for_quality,
};
use crate::domain::snapshot::BURST_INTERVAL_MS;
use crate::domain::timeline::{EventType, TimelineEvent};
use crate::domain::view;
use crate::infrastructure::audio::{build_audio_pipeline_for_url, poll_level_bus};
use crate::infrastructure::reconnect::ReconnectDecision;
use crate::infrastructure::view_state::ViewStateFile;

use super::app::PendingBurst;
use super::bridge::now_unix_secs;
use super::message::LayoutMode;
use super::sidebar;
use super::state::{BackoffState, Toast, VU_PEAK_DECAY_MS};
use super::{App, Message};

/// Focus target used to blur the search box: focusing an id that no widget
/// owns makes iced unfocus everything.
const BLUR_TARGET: &str = "__rrv_blur__";

/// A freshly (re)started pipeline shows `CONNECTING` rather than
/// `RECONNECTING` / `OFFLINE` for this long, so the grid doesn't flap amber
/// while a dozen HLS feeds hand-shake.
const CONNECT_GRACE_SECS: u64 = 12;

pub fn update(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::FrameUpdate => update_frame(app),
        Message::KeyPressed(key, modifiers) => handle_key(app, key, modifiers),
        Message::OpenDir(dir) => {
            crate::infrastructure::notify::open_dir(&dir);
            Task::none()
        }
        Message::EventClicked(idx) => {
            if idx < app.videos.len() {
                app.sidebar.selected = Some(idx);
                app.flex_main_idx = idx;
                note_interaction(app);
                sync_active_streams(app);
            }
            Task::none()
        }
        Message::EditZones(idx) => {
            if idx >= app.videos.len() {
                return Task::none();
            }
            app.context_menu = None;
            app.zone_edit = Some(super::app::ZoneEdit {
                camera_idx: idx,
                temp_vertices: Vec::new(),
            });
            toast(
                app,
                "Zonas: clique para marcar os pontos · Enter conclui · Esc sai",
            );
            update(app, Message::EnterSpotlight(idx))
        }
        Message::Noop => Task::none(),
        Message::ZoneVertex(x, y) => {
            if let Some(edit) = app.zone_edit.as_mut() {
                let p = crate::domain::zones::Point::new(x, y);
                // A double-click would otherwise stack two identical vertices.
                let duplicate = edit
                    .temp_vertices
                    .last()
                    .is_some_and(|l| (l.x - p.x).hypot(l.y - p.y) < MIN_VERTEX_GAP);
                if !duplicate {
                    edit.temp_vertices.push(p);
                }
            }
            Task::none()
        }
        Message::ZoneFinish => {
            finish_zone(app);
            Task::none()
        }
        Message::ZoneUndo => {
            undo_zone(app);
            Task::none()
        }
        Message::ZoneClear => {
            if let Some(idx) = app.zone_edit.as_ref().map(|e| e.camera_idx) {
                if let Some(cfg) = app.engine.zones.get_mut(idx) {
                    cfg.zones.clear();
                }
                persist_zones(app, idx);
                toast(app, "Zonas removidas: o quadro inteiro conta");
            }
            Task::none()
        }
        Message::ZoneCancel => {
            app.zone_edit = None;
            Task::none()
        }
        Message::ThemeChanged(t) => {
            app.theme = t;
            Task::none()
        }
        Message::Sidebar(msg) => update_sidebar(app, msg),
        Message::LayoutModeChanged(mode) => {
            app.layout_mode = mode;
            sync_active_streams(app);
            persist_view(app);
            Task::none()
        }
        Message::FlexMainSelected(idx) => {
            if idx < app.videos.len() {
                app.flex_main_idx = idx;
                sync_active_streams(app);
            }
            Task::none()
        }
        Message::GridModeChanged(mode) => {
            app.view.mode = mode;
            clamp_current_page(app);
            note_interaction(app);
            sync_active_streams(app);
            persist_view(app);
            Task::none()
        }
        Message::NextPage | Message::PrevPage => {
            let pc = grid_page_count(app);
            app.current_page = match message {
                Message::NextPage => view::next_page(app.current_page, pc),
                _ => view::prev_page(app.current_page, pc),
            };
            note_interaction(app);
            sync_active_streams(app);
            Task::none()
        }
        Message::ToggleRotate => {
            app.view.rotate_enabled = !app.view.rotate_enabled;
            app.rotate_last_advance = Instant::now();
            app.interaction_pause_until = None;
            sync_active_streams(app);
            persist_view(app);
            toast(
                app,
                if app.view.rotate_enabled {
                    format!("Carrossel ligado ({}s)", app.view.rotate_secs)
                } else {
                    "Carrossel desligado".to_string()
                },
            );
            Task::none()
        }
        Message::RotateIntervalStep(up) => {
            app.view.rotate_secs = view::step_rotate_secs(app.view.rotate_secs, up);
            app.rotate_last_advance = Instant::now();
            persist_view(app);
            toast(app, format!("Carrossel: {}s", app.view.rotate_secs));
            Task::none()
        }
        Message::WindowResized(size) => {
            app.window_size = size;
            Task::none()
        }
        Message::Snapshot => update_snapshot(app),
        Message::SnapshotSaved {
            camera_idx,
            sequence,
            result,
        } => {
            match result {
                Ok(path) => {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    log::info!("Snapshot saved: {}", path.display());
                    push_event(app, camera_idx, EventType::Snapshot, Some(name.clone()));
                    // Only the lone snapshot / first burst frame toasts on
                    // success; the burst reports its own "complete" toast.
                    if sequence == 1 && app.pending_burst.is_none() {
                        app.toasts.push(Toast {
                            message: format!("Snapshot: {name} · clique para abrir a pasta"),
                            shown_at: Instant::now(),
                            open_dir: path.parent().map(std::path::Path::to_path_buf),
                        });
                    }
                }
                Err(e) => {
                    log::warn!("Falha no snapshot: {e}");
                    toast(app, format!("Falha no snapshot: {e}"));
                }
            }
            Task::none()
        }
        Message::ToggleRecording => update_recording(app),
        Message::ToggleAudio => update_audio(app),
        Message::VolumeUp => update_volume(app, true),
        Message::VolumeDown => update_volume(app, false),
        Message::ToggleSelection => {
            if app.sidebar.selected.is_some() {
                app.sidebar.selected = None;
            } else if !app.videos.is_empty() {
                app.sidebar.selected = Some(0);
            }
            Task::none()
        }
        Message::CycleLayout => {
            app.layout_mode = app.layout_mode.next();
            sync_active_streams(app);
            persist_view(app);
            Task::none()
        }
        Message::SelectCamera(idx) => {
            if idx < app.videos.len() {
                // Second click on the same cell within DOUBLE_CLICK_MS opens
                // that camera's spotlight.
                let double = matches!(
                    app.last_cell_click,
                    Some((prev, t))
                        if prev == idx
                            && t.elapsed().as_millis() <= super::app::DOUBLE_CLICK_MS
                );
                app.sidebar.selected = Some(idx);
                app.flex_main_idx = idx;
                sync_active_streams(app);
                if double {
                    return update(app, Message::EnterSpotlight(idx));
                }
                app.last_cell_click = Some((idx, Instant::now()));
            }
            Task::none()
        }
        Message::ToggleSidebar => {
            app.sidebar.visible = !app.sidebar.visible;
            persist_view(app);
            Task::none()
        }
        Message::CycleSidebarTab => {
            app.sidebar.visible = true;
            app.sidebar.active_view = match app.sidebar.active_view {
                sidebar::SidebarView::Cameras => sidebar::SidebarView::Info,
                sidebar::SidebarView::Info => sidebar::SidebarView::Diagnostics,
                sidebar::SidebarView::Diagnostics => sidebar::SidebarView::Timeline,
                sidebar::SidebarView::Timeline => sidebar::SidebarView::Cameras,
            };
            Task::none()
        }
        Message::ToggleFullscreen => {
            let entering = !app.is_fullscreen;
            let new_mode = if entering {
                iced::window::Mode::Fullscreen
            } else {
                iced::window::Mode::Windowed
            };
            // Optimistic: assumes the window manager honours `change_mode`.
            // On a WM that ignores it (some tiling/headless setups) the flag
            // desyncs and Esc/ExitFullscreen won't recover until the next
            // toggle.
            app.is_fullscreen = entering;
            // OS fullscreen and immersive go together: a fullscreen window that
            // still shows the toolbar/sidebar isn't really "fullscreen video".
            if entering {
                if app.focus.is_normal() {
                    app.focus = super::app::ViewFocus::Immersive;
                }
            } else if app.focus == super::app::ViewFocus::Immersive {
                app.focus = super::app::ViewFocus::Normal;
            }
            reveal_chrome(app);
            iced::window::get_latest().and_then(move |id| iced::window::change_mode(id, new_mode))
        }
        Message::ExitFullscreen => {
            if app.is_fullscreen {
                app.is_fullscreen = false;
                if app.focus == super::app::ViewFocus::Immersive {
                    app.focus = super::app::ViewFocus::Normal;
                }
                return iced::window::get_latest()
                    .and_then(|id| iced::window::change_mode(id, iced::window::Mode::Windowed));
            }
            Task::none()
        }
        Message::ToggleImmersive => {
            app.focus = if app.focus.is_normal() {
                super::app::ViewFocus::Immersive
            } else {
                super::app::ViewFocus::Normal
            };
            app.show_overflow_menu = false;
            reveal_chrome(app);
            Task::none()
        }
        Message::EnterSpotlight(idx) => {
            if idx < app.videos.len() {
                app.sidebar.selected = Some(idx);
                app.flex_main_idx = idx;
                app.focus = super::app::ViewFocus::Spotlight(idx);
                app.last_cell_click = None;
                reveal_chrome(app);
                sync_active_streams(app);
            }
            Task::none()
        }
        Message::SpotlightStep(forward) => {
            app.zone_edit = None;
            if let super::app::ViewFocus::Spotlight(cur) = app.focus {
                let ordered = ordered_visible_cameras(app);
                if let Some(pos) = ordered.iter().position(|&i| i == cur) {
                    let n = ordered.len();
                    let next = if forward {
                        ordered[(pos + 1) % n]
                    } else {
                        ordered[(pos + n - 1) % n]
                    };
                    app.focus = super::app::ViewFocus::Spotlight(next);
                    app.sidebar.selected = Some(next);
                    app.flex_main_idx = next;
                    reveal_chrome(app);
                    sync_active_streams(app);
                }
            }
            Task::none()
        }
        Message::ExitFocus => {
            app.zone_edit = None;
            app.focus = super::app::ViewFocus::Normal;
            app.show_overflow_menu = false;
            sync_active_streams(app);
            Task::none()
        }
        Message::RevealChrome => {
            reveal_chrome(app);
            Task::none()
        }
        Message::ToggleOverflowMenu => {
            app.show_overflow_menu = !app.show_overflow_menu;
            Task::none()
        }
        Message::CellHoverEnter(idx) => {
            app.hovered_cell = Some(idx);
            Task::none()
        }
        Message::CellHoverExit => {
            app.hovered_cell = None;
            Task::none()
        }
        Message::PointerMoved(p) => {
            app.pointer_pos = p;
            Task::none()
        }
        Message::RunMenuCommand(inner) => {
            app.show_overflow_menu = false;
            app.context_menu = None;
            update(app, *inner)
        }
        Message::Quit => {
            shutdown(app);
            iced::window::get_latest().and_then(iced::window::close)
        }
        Message::ShowHelp => {
            app.show_help = !app.show_help;
            Task::none()
        }
        Message::ShowContextMenu(idx) => {
            if idx < app.videos.len() {
                app.sidebar.selected = Some(idx);
                app.context_menu = Some(super::state::ContextMenu {
                    camera_idx: idx,
                    anchor: app.pointer_pos,
                });
            }
            Task::none()
        }
        Message::DismissContextMenu => {
            app.context_menu = None;
            Task::none()
        }
        Message::FocusSearch => {
            app.search_focused = true;
            app.sidebar.visible = true;
            app.sidebar.active_view = sidebar::SidebarView::Cameras;
            iced::widget::text_input::focus(sidebar::cameras::search_input_id())
        }
        Message::BlurSearch => {
            app.search_focused = false;
            iced::widget::text_input::focus(iced::widget::text_input::Id::new(BLUR_TARGET))
        }
    }
}

/// Keys that close the (modal) help popup.
fn is_help_dismiss(key: &iced::keyboard::Key) -> bool {
    use iced::keyboard::Key;
    use iced::keyboard::key::Named;
    matches!(
        key.as_ref(),
        Key::Named(Named::Escape) | Key::Character("?")
    )
}

/// Resolve a raw key press against the current focus state.
///
/// This runs in `update` rather than the subscription because iced identifies
/// a subscription by its closure type: a captured "is the search box focused"
/// flag would be baked in at first subscribe and never refresh.
fn handle_key(
    app: &mut App,
    key: iced::keyboard::Key,
    modifiers: iced::keyboard::Modifiers,
) -> Task<Message> {
    use iced::keyboard::Key;
    use iced::keyboard::key::Named;

    // Quitting is destructive and must never be one stray keystroke away
    // while the user is typing, so it lives behind a modifier.
    if modifiers.command() {
        return match key.as_ref() {
            Key::Character("q") => update(app, Message::Quit),
            _ => Task::none(),
        };
    }

    // The help popup is modal: only its own toggle / Esc act, so `f`, `s` or
    // `r` cannot fire on the UI hidden behind the scrim.
    if app.show_help {
        return if is_help_dismiss(&key) {
            update(app, Message::ShowHelp)
        } else {
            Task::none()
        };
    }

    // While drawing zones the keyboard belongs to the editor: Enter closes the
    // polygon, Backspace undoes, Esc leaves. Nothing else may fire (`f` would
    // otherwise drop out of the spotlight the editor is drawn on).
    if app.zone_edit.is_some() {
        match key.as_ref() {
            Key::Named(Named::Enter) => return update(app, Message::ZoneFinish),
            Key::Named(Named::Backspace) => return update(app, Message::ZoneUndo),
            Key::Named(Named::Escape) => return update(app, Message::ZoneCancel),
            _ => {}
        }
    }

    // Function keys and Escape are unambiguous — a text field never wants
    // them — so they work regardless of focus.
    match key.as_ref() {
        Key::Named(Named::F12) => return update(app, Message::Snapshot),
        Key::Named(Named::F2) => return update(app, Message::ToggleSidebar),
        Key::Named(Named::F3) => return update(app, Message::CycleSidebarTab),
        Key::Named(Named::F11) => return update(app, Message::ToggleFullscreen),
        Key::Named(Named::Escape) => {
            use super::app::ViewFocus;
            if app.show_overflow_menu {
                app.show_overflow_menu = false;
                return Task::none();
            }
            if app.context_menu.is_some() {
                app.context_menu = None;
                return Task::none();
            }
            if app.search_focused {
                return update(app, Message::BlurSearch);
            }
            match app.focus {
                ViewFocus::Spotlight(_) => return update(app, Message::ExitFocus),
                ViewFocus::Immersive => {
                    app.focus = ViewFocus::Normal;
                    return update(app, Message::ExitFullscreen);
                }
                ViewFocus::Normal => return update(app, Message::ExitFullscreen),
            }
        }
        _ => {}
    }

    // Any key press keeps the immersive reveal rail alive for a beat.
    if !app.focus.is_normal() {
        reveal_chrome(app);
    }

    // Everything below is a bare single-key shortcut. While the search box
    // has the keyboard, those keystrokes belong to it.
    if app.search_focused || app.zone_edit.is_some() || modifiers.control() || modifiers.alt() {
        return Task::none();
    }

    match key.as_ref() {
        Key::Named(Named::Space) => update(app, Message::ToggleSelection),
        Key::Named(Named::Tab) => update(app, Message::CycleLayout),
        Key::Named(Named::PageDown) => update(app, Message::NextPage),
        Key::Named(Named::PageUp) => update(app, Message::PrevPage),
        Key::Named(Named::Enter) => match app.sidebar.selected {
            Some(idx) => update(app, Message::EnterSpotlight(idx)),
            None => Task::none(),
        },
        Key::Named(Named::ArrowRight) if !app.focus.is_normal() => {
            update(app, Message::SpotlightStep(true))
        }
        Key::Named(Named::ArrowLeft) if !app.focus.is_normal() => {
            update(app, Message::SpotlightStep(false))
        }
        Key::Character("]") => update(app, Message::NextPage),
        Key::Character("[") => update(app, Message::PrevPage),
        Key::Character("c") => update(app, Message::ToggleRotate),
        Key::Character("h") => update(app, Message::ToggleImmersive),
        Key::Character("f") => match app.sidebar.selected {
            Some(idx) if app.focus.is_normal() => update(app, Message::EnterSpotlight(idx)),
            _ => update(app, Message::ExitFocus),
        },
        Key::Character("g") => {
            let next = app.view.mode.cycle();
            update(app, Message::GridModeChanged(next))
        }
        Key::Character("/") => update(app, Message::FocusSearch),
        Key::Character("k") => update(app, Message::ToggleSelection),
        Key::Character("s") => update(app, Message::Snapshot),
        Key::Character("r") => update(app, Message::ToggleRecording),
        Key::Character("m") => update(app, Message::ToggleAudio),
        Key::Character("+") | Key::Character("=") => update(app, Message::VolumeUp),
        Key::Character("-") => update(app, Message::VolumeDown),
        Key::Character("?") => update(app, Message::ShowHelp),
        Key::Character(c)
            if c.chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_digit() && ch != '0') =>
        {
            // Guard above guarantees a 1-9 digit; `map_or` keeps this panic-free.
            let n = c
                .chars()
                .next()
                .and_then(|ch| ch.to_digit(10))
                .map_or(0, |d| d as usize);
            update(app, Message::SelectCamera(n.saturating_sub(1)))
        }
        _ => Task::none(),
    }
}

/// Tear every pipeline down explicitly so in-flight recordings are finalised
/// before the process exits.
fn shutdown(app: &mut App) {
    for pipeline_rc in &app.audio_pipelines {
        if let Some(pipeline) = pipeline_rc.borrow_mut().take() {
            let _ = pipeline.set_state(gstreamer::State::Null);
        }
    }
    for bridge in &app.engine.bridges {
        // Blocking: the process is about to exit, so any recording must be
        // finalised now rather than on a thread that won't survive.
        bridge
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .stop_blocking();
    }
}

/// Closest two consecutive vertices may be, in normalized units (1% of the frame).
const MIN_VERTEX_GAP: f64 = 0.01;
/// Smallest polygon worth saving (0.05% of the frame).
const MIN_ZONE_AREA: f64 = 0.0005;

/// Commit the polygon being drawn as a new zone of the edited camera.
fn finish_zone(app: &mut App) {
    let Some(edit) = app.zone_edit.as_mut() else {
        return;
    };
    if edit.temp_vertices.len() < 3 {
        toast(app, "Uma zona precisa de pelo menos 3 pontos");
        return;
    }
    if crate::domain::zones::polygon_area(&edit.temp_vertices) < MIN_ZONE_AREA {
        toast(app, "Zona sem área: os pontos estão alinhados ou repetidos");
        return;
    }
    let idx = edit.camera_idx;
    let vertices = std::mem::take(&mut edit.temp_vertices);
    if let Some(cfg) = app.engine.zones.get_mut(idx) {
        let name = format!("Zona {}", cfg.zones.len() + 1);
        cfg.zones
            .push(crate::domain::zones::MotionZone::new(name, vertices));
    }
    persist_zones(app, idx);
    toast(app, "Zona salva");
}

/// Backspace: drop the last vertex; with no open polygon, the last saved zone.
fn undo_zone(app: &mut App) {
    let Some(edit) = app.zone_edit.as_mut() else {
        return;
    };
    if edit.temp_vertices.pop().is_some() {
        return;
    }
    let idx = edit.camera_idx;
    if app
        .engine
        .zones
        .get_mut(idx)
        .is_some_and(|c| c.zones.pop().is_some())
    {
        persist_zones(app, idx);
    }
}

fn persist_zones(app: &mut App, idx: usize) {
    let (Some(cam), Some(cfg)) = (app.sidebar.cameras.get(idx), app.engine.zones.get(idx)) else {
        return;
    };
    app.zones_file.set(&cam.name, cfg);
    crate::infrastructure::zone_state::save(&app.zones_file);
}

/// Sample the camera's latest frame and compare it with the previous sample.
/// Called at ~2 Hz per live camera; logs a timeline event on the rising edge.
fn detect_camera_motion(app: &mut App, i: usize) {
    if !app.engine.motion_config.enabled
        || !matches!(
            app.sidebar.cameras[i].status,
            sidebar::CameraStatus::Live | sidebar::CameraStatus::Recording
        )
    {
        app.engine.prev_motion_frames[i] = None;
        app.engine.motion_active[i] = false;
        return;
    }
    // The reduced detection branch (~320 px), not the full-resolution display
    // frame: same answer for a fraction of the pixels.
    let frame = app.engine.bridges[i]
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .capture_detect_frame();
    let Some((curr, width, height)) = frame else {
        return;
    };
    // Same allocation as last time = the branch has not produced a new frame
    // yet; diffing a frame with itself would read as "stillness".
    if app.engine.prev_motion_frames[i]
        .as_ref()
        .is_some_and(|prev| prev.as_ptr() == curr.as_ptr() && prev.len() == curr.len())
    {
        return;
    }
    let zones = app.engine.zones.get(i).filter(|z| z.has_active());
    let result = app.engine.prev_motion_frames[i].as_ref().and_then(|prev| {
        crate::domain::motion::detect_motion(
            prev,
            &curr,
            width as usize,
            height as usize,
            &app.engine.motion_config,
            zones,
        )
    });
    app.engine.prev_motion_frames[i] = Some(curr);
    let Some(result) = result else {
        return;
    };
    log::debug!(
        "Motion sample, camera {i}: {:.1}% changed{}",
        result.motion_level * 100.0,
        if result.global_change {
            " (global change, discarded)"
        } else {
            ""
        }
    );
    if result.motion_active && !app.engine.motion_active[i] {
        log::info!(
            "Motion on camera {i}: {:.1}% of the frame",
            result.motion_level * 100.0
        );
        push_event(
            app,
            i,
            EventType::Motion,
            Some(format!("{:.1}% do quadro", result.motion_level * 100.0)),
        );
    }
    app.engine.motion_active[i] = result.motion_active;
}

/// Fire a desktop notification for motion / offline events, at most once per
/// cooldown per camera and kind.
fn notify_desktop(app: &mut App, camera_idx: usize, kind: EventType, detail: Option<&str>) {
    if !app.engine.notify.enabled {
        return;
    }
    let name = app
        .sidebar
        .cameras
        .get(camera_idx)
        .map_or_else(|| format!("Câmera {}", camera_idx + 1), |c| c.name.clone());
    let Some((title, body)) = crate::domain::notify::message_for(kind, &name, detail) else {
        return;
    };
    let key = (camera_idx, kind.label());
    let since = app
        .engine
        .notify_last
        .get(&key)
        .map(|t| t.elapsed().as_secs());
    if !crate::domain::notify::cooldown_elapsed(since, app.engine.notify.cooldown_secs) {
        return;
    }
    app.engine.notify_last.insert(key, Instant::now());
    crate::infrastructure::notify::send(&title, &body);
}

fn push_event(app: &mut App, camera_idx: usize, kind: EventType, description: Option<String>) {
    notify_desktop(app, camera_idx, kind, description.as_deref());
    let mut event = TimelineEvent::new(now_unix_secs(), camera_idx, kind);
    if let Some(d) = description {
        event = event.with_description(d);
    }
    app.sidebar.timeline.push(event);
}

fn toast(app: &mut App, message: impl Into<String>) {
    app.toasts.push(Toast {
        message: message.into(),
        shown_at: Instant::now(),
        open_dir: None,
    });
}

// ─────────────────────────── view-mode helpers ───────────────────────────

/// Show the immersive/spotlight reveal rail and (re)arm its auto-hide timer.
fn reveal_chrome(app: &mut App) {
    app.chrome_revealed = true;
    app.chrome_revealed_at = Instant::now();
}

/// Pause the carousel for a beat after a manual navigation so the user can
/// look at what they picked.
fn note_interaction(app: &mut App) {
    app.interaction_pause_until =
        Some(Instant::now() + Duration::from_secs(view::INTERACTION_PAUSE_SECS));
}

/// Visible camera indices (group filter applied) in the user's saved order.
fn ordered_visible_cameras(app: &App) -> Vec<usize> {
    let visible = app.sidebar.visible_camera_indices(app.videos.len());
    view::apply_order(&app.view.order, &visible)
}

fn grid_page_size(app: &App) -> usize {
    app.view.mode.page_size(ordered_visible_cameras(app).len())
}

fn grid_page_count(app: &App) -> usize {
    view::page_count(ordered_visible_cameras(app).len(), grid_page_size(app))
}

fn clamp_current_page(app: &mut App) {
    app.current_page = view::clamp_page(app.current_page, grid_page_count(app));
}

/// The set of camera indices whose pipelines should be running right now.
///
/// * `pause_hidden` off, or motion recording / alerts on
///   ([`crate::domain::motion::needs_background_watch`]) → every enabled camera.
/// * Flex layout → the main camera (thumbnails are stills).
/// * Grid layout with `pause_hidden` → the current page, plus the next page
///   as a prefetch when the carousel is on or [`view::PREFETCH_NEXT_PAGE`].
fn desired_active_cameras(app: &App) -> Vec<usize> {
    let enabled_all: Vec<usize> = (0..app.engine.bridges.len())
        .filter(|&i| app.engine.camera_enabled[i])
        .collect();

    if !app.engine.pause_hidden {
        return enabled_all;
    }

    // A hidden camera is blind: no motion, so no motion recording or alert.
    // When something reacts to motion, every camera keeps decoding (on its
    // sub-stream when it has one, see `wanted_quality`).
    if crate::domain::motion::needs_background_watch(
        app.engine.motion_config.enabled,
        app.engine.motion_recording,
        app.engine.notify.enabled,
    ) {
        return enabled_all;
    }

    // Flex shows one live camera; the thumbnail strip stays paused until a
    // thumbnail is picked, so only the camera on screen holds a connection.
    if app.layout_mode == LayoutMode::Flex {
        let mut hot = app
            .flex_main_idx
            .min(app.engine.bridges.len().saturating_sub(1));
        if let super::app::ViewFocus::Spotlight(idx) = app.focus {
            hot = idx;
        }
        let preview = app.preview_cam.map(|(i, _)| i);
        return enabled_all
            .into_iter()
            .filter(|&i| i == hot || Some(i) == preview)
            .collect();
    }

    let ordered = ordered_visible_cameras(app);
    let page_size = grid_page_size(app);
    let page_count = view::page_count(ordered.len(), page_size);
    let page = view::clamp_page(app.current_page, page_count);

    let mut want: Vec<usize> = ordered[view::page_slice(page, page_size, ordered.len())].to_vec();
    if view::PREFETCH_NEXT_PAGE || app.view.rotate_enabled {
        let next = view::next_page(page, page_count);
        if next != page {
            want.extend_from_slice(&ordered[view::page_slice(next, page_size, ordered.len())]);
        }
    }
    // Keep the selected / spotlighted camera hot even if it scrolled off-page
    // (its metrics panel and any audio would otherwise die under the user, and
    // spotlight needs it decoding).
    let mut hot = app.sidebar.selected;
    if let super::app::ViewFocus::Spotlight(idx) = app.focus {
        hot = Some(idx);
    }
    if let Some(sel) = hot
        && app.engine.camera_enabled.get(sel).copied().unwrap_or(false)
        && !want.contains(&sel)
    {
        want.push(sel);
    }
    want.retain(|&i| app.engine.camera_enabled.get(i).copied().unwrap_or(false));
    want.sort_unstable();
    want.dedup();
    want
}

/// Start a camera's video pipeline now (used by the staggered drain), on the
/// stream its current view calls for.
fn start_stream(app: &mut App, i: usize) {
    let quality = wanted_quality(app, i);
    restart_stream(app, i, quality);
}

/// (Re)build a camera's pipeline on `quality`. `start_*` stops the old
/// pipeline first, so this doubles as the sub/main switch.
fn restart_stream(app: &mut App, i: usize, quality: StreamQuality) {
    if i >= app.engine.stream_quality.len() {
        return;
    }
    app.engine.stream_quality[i] = quality;
    let Some(cfg) = camera_config_for(app, i) else {
        return;
    };
    let result = {
        let mut bridge = app.engine.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        bridge.start_from_config(&cfg)
    };
    app.engine.active_stream[i] = true;
    app.engine.connecting_since[i] = Some(Instant::now());
    app.engine.reconnect_states[i].reset();
    app.engine.backoff_states[i] = BackoffState::new();
    if i < app.sidebar.cameras.len() {
        app.sidebar.cameras[i].status = if result.is_err() {
            sidebar::CameraStatus::Offline
        } else {
            sidebar::CameraStatus::Connecting
        };
    }
    if let Err(e) = result {
        log::warn!("Could not start camera {i}: {e}");
    }
}

/// The camera config with `url` swapped for the sub-stream when that is the
/// stream it is running on.
fn camera_config_for(app: &App, i: usize) -> Option<crate::config::CameraConfig> {
    let mut cfg = app.engine.camera_configs.get(i)?.clone();
    let multi = MultiStreamConfig {
        sub_stream_url: cfg.sub_url.clone(),
        default_quality: StreamQuality::Main,
    };
    let quality = app
        .engine
        .stream_quality
        .get(i)
        .copied()
        .unwrap_or_default();
    cfg.url = stream_url_for_quality(&cfg.url, quality, &multi);
    Some(cfg)
}

/// The stream camera `i` should be on given what is on screen.
fn wanted_quality(app: &App, i: usize) -> StreamQuality {
    let current = app
        .engine
        .stream_quality
        .get(i)
        .copied()
        .unwrap_or_default();
    let has_sub = app
        .engine
        .camera_configs
        .get(i)
        .is_some_and(|c| c.sub_url.is_some());
    if !has_sub {
        return StreamQuality::Main;
    }
    let recording = app.engine.bridges[i]
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_recording();
    desired_quality(has_sub, recording, current, is_large_view(app, i))
}

/// Does camera `i` fill the view (spotlight, flex main, or alone on the page)?
fn is_large_view(app: &App, i: usize) -> bool {
    if matches!(app.focus, super::app::ViewFocus::Spotlight(idx) if idx == i) {
        return true;
    }
    if app.layout_mode == LayoutMode::Flex {
        return i == app.flex_main_idx;
    }
    ordered_visible_cameras(app).len() <= 1
}

/// Tear a camera's video + audio pipeline down because it went off-page.
fn pause_stream(app: &mut App, i: usize) {
    {
        let mut bridge = app.engine.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        bridge.stop();
    }
    if let Some(pipeline) = app.audio_pipelines[i].borrow_mut().take() {
        let _ = pipeline.set_state(gstreamer::State::Null);
    }
    app.audio_states[i] = AudioState::Muted;
    app.engine.active_stream[i] = false;
    app.engine.connecting_since[i] = None;
    app.engine.reconnect_states[i].reset();
    app.engine.backoff_states[i] = BackoffState::new();
    app.engine.start_queue.retain(|&q| q != i);
    if i < app.sidebar.cameras.len() {
        app.sidebar.cameras[i].status = sidebar::CameraStatus::Paused;
    }
}

/// Reconcile running pipelines with [`desired_active_cameras`]. Cameras that
/// should run but don't are queued for the staggered drain; cameras that run
/// but shouldn't are stopped immediately.
fn sync_active_streams(app: &mut App) {
    clamp_current_page(app);
    let want = desired_active_cameras(app);

    for i in 0..app.engine.bridges.len() {
        if !app.engine.camera_enabled[i] {
            continue;
        }
        let should_run = want.contains(&i);
        if should_run && !app.engine.active_stream[i] {
            if !app.engine.start_queue.contains(&i) {
                app.engine.start_queue.push_back(i);
            }
            if i < app.sidebar.cameras.len()
                && app.sidebar.cameras[i].status == sidebar::CameraStatus::Paused
            {
                app.sidebar.cameras[i].status = sidebar::CameraStatus::Connecting;
            }
        } else if !should_run && app.engine.active_stream[i] {
            pause_stream(app, i);
        } else if !should_run && !app.engine.active_stream[i] {
            // Off-page and not running: settle its placeholder on PAUSED
            // (unless it never started and is still in the launch queue).
            app.engine.start_queue.retain(|&q| q != i);
            if i < app.sidebar.cameras.len()
                && app.sidebar.cameras[i].status == sidebar::CameraStatus::Connecting
            {
                app.sidebar.cameras[i].status = sidebar::CameraStatus::Paused;
            }
        }
    }

    // Running cameras whose view changed (grid tile ↔ spotlight) swap stream.
    for i in 0..app.engine.bridges.len() {
        if !app.engine.camera_enabled[i] || !app.engine.active_stream[i] {
            continue;
        }
        let wanted = wanted_quality(app, i);
        if wanted != app.engine.stream_quality[i] {
            log::info!("Camera {i}: switching to the {} stream", wanted.label());
            restart_stream(app, i, wanted);
        }
    }
}

/// Flex thumbnails are stills: connect each non-main camera just long enough
/// to grab one frame, then pause it again. One camera at a time, and only
/// once the launch queue is empty so it never competes with the main stream.
fn drive_previews(app: &mut App) {
    const PREVIEW_TIMEOUT_SECS: u64 = 10;
    if app.layout_mode != LayoutMode::Flex || !app.engine.pause_hidden {
        return;
    }
    if let Some((i, since)) = app.preview_cam {
        let got = app.engine.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .capture_frame()
            .is_some();
        if got || since.elapsed().as_secs() >= PREVIEW_TIMEOUT_SECS {
            app.preview_done[i] = true;
            app.preview_cam = None;
            if i != app.flex_main_idx {
                pause_stream(app, i);
            }
        }
        return;
    }
    if !app.engine.start_queue.is_empty() || Instant::now() < app.engine.next_start_at {
        return;
    }
    let next = (0..app.engine.bridges.len()).find(|&i| {
        app.engine.camera_enabled[i]
            && !app.engine.active_stream[i]
            && !app.preview_done[i]
            && app.engine.bridges[i]
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .capture_frame()
                .is_none()
    });
    if let Some(i) = next {
        app.preview_cam = Some((i, Instant::now()));
        start_stream(app, i);
        app.engine.next_start_at = Instant::now() + app.engine.stagger;
    }
}

/// Start at most one queued camera per `stagger`.
fn drain_start_queue(app: &mut App) {
    if app.engine.start_queue.is_empty() || Instant::now() < app.engine.next_start_at {
        return;
    }
    let want = desired_active_cameras(app);
    while let Some(i) = app.engine.start_queue.pop_front() {
        if !app.engine.camera_enabled.get(i).copied().unwrap_or(false)
            || app.engine.active_stream[i]
        {
            continue;
        }
        if !want.contains(&i) {
            // No longer needed (page moved again before its turn came up).
            continue;
        }
        start_stream(app, i);
        app.engine.next_start_at = Instant::now() + app.engine.stagger;
        break;
    }
}

/// Advance the page carousel when it is enabled, there is more than one page,
/// no interaction pause is in effect, and the dwell time has elapsed.
fn advance_carousel(app: &mut App) {
    if !app.view.rotate_enabled || app.layout_mode != LayoutMode::Grid {
        return;
    }
    if let Some(until) = app.interaction_pause_until {
        if Instant::now() < until {
            return;
        }
        app.interaction_pause_until = None;
    }
    let page_count = grid_page_count(app);
    if page_count <= 1 {
        return;
    }
    if app.rotate_last_advance.elapsed().as_secs() >= app.view.rotate_secs {
        app.current_page = view::next_page(app.current_page, page_count);
        app.rotate_last_advance = Instant::now();
    }
}

/// Snapshot the persisted view state and write it out (best-effort).
fn persist_view(app: &App) {
    let state = ViewStateFile {
        mode: Some(app.view.mode.as_str()),
        rotate_enabled: Some(app.view.rotate_enabled),
        rotate_secs: Some(app.view.rotate_secs),
        order: Some(app.view.order.clone()),
        active_group: app.sidebar.active_group,
        layout: Some(
            match app.layout_mode {
                LayoutMode::Grid => "grid",
                LayoutMode::Flex => "flex",
            }
            .to_string(),
        ),
        sidebar_visible: Some(app.sidebar.visible),
    };
    crate::infrastructure::view_state::save(&state);
}

fn update_frame(app: &mut App) -> Task<Message> {
    app.toasts.retain(|t| !super::state::is_expired(t));

    // Latency / RTP-stats don't need 10 Hz; poll them every 5th tick (~500 ms)
    // to keep the per-camera locked section short.
    app.tick_count = app.tick_count.wrapping_add(1);
    let poll_slow_metrics = app.tick_count.is_multiple_of(5);

    // View-mode housekeeping: advance the carousel, reconcile which pipelines
    // should be running for the current page, then start at most one queued
    // camera this tick (staggered so a dozen feeds don't connect at once).
    advance_carousel(app);
    drive_previews(app);
    sync_active_streams(app);
    drain_start_queue(app);

    // Auto-hide the immersive reveal rail after a few idle seconds.
    if app.chrome_revealed
        && app.chrome_revealed_at.elapsed().as_secs() >= super::app::CHROME_REVEAL_SECS
    {
        app.chrome_revealed = false;
    }

    for i in 0..app.engine.bridges.len() {
        // A camera that is disabled, paused off-page, or still waiting in the
        // start queue has no live pipeline — skip the bus/FPS/reconnect work
        // and leave its placeholder status untouched.
        if !app.engine.camera_enabled[i] || !app.engine.active_stream[i] {
            continue;
        }

        let was_offline = matches!(
            app.sidebar.cameras[i].status,
            sidebar::CameraStatus::Offline | sidebar::CameraStatus::Reconnecting
        );

        let cam_label = app
            .sidebar
            .cameras
            .get(i)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("Camera {}", i + 1));

        // Everything that needs the bridge happens inside this block. The
        // guard must be released before the reconnect path below re-locks the
        // same mutex — `std::sync::Mutex` is not reentrant, so holding it
        // across both deadlocks the UI thread permanently.
        let (needs_reconnect, errored) = {
            let mut bridge = app.engine.bridges[i]
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            bridge.poll_bus();
            let fps = bridge.update_fps();
            app.fps_history[i].push(fps);
            if app.fps_history[i].len() > 60 {
                app.fps_history[i].remove(0);
            }
            let metrics = bridge.metrics().clone();
            metrics
                .current_fps_x1000
                .store((fps * 1000.0) as u64, Ordering::Relaxed);
            metrics.is_live.store(bridge.is_live(), Ordering::Relaxed);
            bridge.discover_rtp_jitterbuffers();
            bridge.discover_decoder();
            if poll_slow_metrics {
                bridge.query_latency();
                bridge.poll_rtp_stats();
            }
            if let Some(reading) = bridge.sample_status(&app.sidebar.cameras[i].status) {
                app.sidebar.cameras[i].apply(reading);
            }
            let hist = &mut app.sidebar.cameras[i].fps_history;
            hist.push(fps);
            if hist.len() > 16 {
                hist.remove(0);
            }
            app.sidebar.cameras[i].is_muted = !app
                .audio_states
                .get(i)
                .map(|s| s.is_audible())
                .unwrap_or(false);
            app.sidebar.cameras[i].is_recording = bridge.is_recording();

            let is_live = bridge.is_live();
            let is_uridecodebin = app
                .engine
                .camera_configs
                .get(i)
                .map(|c| {
                    c.use_uridecodebin.unwrap_or(false)
                        || c.url.starts_with("http://")
                        || c.url.starts_with("https://")
                })
                .unwrap_or(false);

            let backoff_due = app.engine.backoff_states[i].is_due();
            let decision = app.engine.reconnect_states[i].tick(
                is_live,
                is_uridecodebin,
                Some(fps),
                backoff_due,
                &cam_label,
            );

            if is_live && fps > 0.0 && matches!(decision, ReconnectDecision::None) {
                app.engine.backoff_states[i].record_success();
            }

            let errored = bridge
                .error_message
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some();
            let reconnect = match decision {
                ReconnectDecision::Reconnect(reason) => {
                    log::info!("[{}] Reconnecting: {}", cam_label, reason);
                    true
                }
                ReconnectDecision::None => false,
            };
            (reconnect, errored)
        };

        // Log connectivity transitions once the status has been refreshed.
        let now_offline = matches!(
            app.sidebar.cameras[i].status,
            sidebar::CameraStatus::Offline | sidebar::CameraStatus::Reconnecting
        );
        if was_offline && !now_offline {
            push_event(app, i, EventType::Online, None);
        } else if !was_offline && now_offline {
            push_event(app, i, EventType::Offline, None);
        }

        // Cosmetic: a pipeline we started in the last few seconds shows
        // CONNECTING rather than flapping to RECONNECTING/OFFLINE while it
        // hand-shakes. Cleared once it actually goes live.
        match app.engine.connecting_since[i] {
            Some(_) if app.sidebar.cameras[i].status == sidebar::CameraStatus::Live => {
                app.engine.connecting_since[i] = None;
            }
            Some(t) if t.elapsed().as_secs() < CONNECT_GRACE_SECS => {
                if matches!(
                    app.sidebar.cameras[i].status,
                    sidebar::CameraStatus::Offline | sidebar::CameraStatus::Reconnecting
                ) {
                    app.sidebar.cameras[i].status = sidebar::CameraStatus::Connecting;
                }
            }
            Some(_) => app.engine.connecting_since[i] = None,
            None => {}
        }

        // Down with no retry pending: schedule one. A pipeline that reported an
        // error is dead, so it retries right away; otherwise wait out the
        // connect grace. Without this a camera that fails right after starting
        // is stuck on "Reconectando" forever (the watchdog only covers live
        // RTSP stalls).
        let down = matches!(
            app.sidebar.cameras[i].status,
            sidebar::CameraStatus::Offline | sidebar::CameraStatus::Reconnecting
        );
        if !needs_reconnect
            && BackoffState::should_schedule_retry(
                app.engine.connecting_since[i].is_some(),
                down,
                errored,
            )
        {
            app.engine.backoff_states[i].arm_if_idle();
        }

        if needs_reconnect {
            reconnect_camera(app, i, &cam_label);
        }

        if poll_slow_metrics {
            detect_camera_motion(app, i);
            drive_motion_recording(app, i);
        }

        // Pull VU levels off the audio pipeline's bus (no GLib loop → no watch).
        if let Some(pl) = app.audio_pipelines[i].borrow().as_ref() {
            poll_level_bus(pl, &app.audio_level_states[i]);
        }

        let raw_level = app
            .audio_level_states
            .get(i)
            .map(|l| l.level())
            .unwrap_or(0.0);
        if raw_level > app.vu_peaks[i] {
            app.vu_peaks[i] = raw_level;
            app.vu_peak_since[i] = Some(Instant::now());
        } else if let Some(peak_time) = app.vu_peak_since[i]
            && peak_time.elapsed().as_millis() > VU_PEAK_DECAY_MS
        {
            app.vu_peaks[i] *= 0.95;
            if app.vu_peaks[i] < 0.01 {
                app.vu_peaks[i] = 0.0;
                app.vu_peak_since[i] = None;
            }
        }
    }

    // `app.is_recording` drives the toolbar Stop button and the grid REC badge
    // for the *selected* camera. Recompute it once here so it stays correct
    // when that camera is disabled (the loop above `continue`s past disabled
    // cameras) or when a different camera is the one recording.
    app.is_recording = app
        .sidebar
        .selected
        .and_then(|i| {
            let enabled = *app.engine.camera_enabled.get(i)?;
            let bridge = app.engine.bridges.get(i)?;
            Some(
                enabled
                    && bridge
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_recording(),
            )
        })
        .unwrap_or(false);

    let burst_task = advance_burst(app);
    update_selected_metrics(app);
    burst_task
}

/// Rebuild a camera's pipeline, preserving an in-progress recording.
fn reconnect_camera(app: &mut App, i: usize, cam_label: &str) {
    let Some(cam_config) = camera_config_for(app, i) else {
        return;
    };

    // Remember whether audio was playing so it can be resumed after the
    // rebuild instead of forcing the user to press `m` on every reconnect.
    let prev_audio = app.audio_states[i];
    if let Some(pipeline) = app.audio_pipelines[i].borrow_mut().take() {
        let _ = pipeline.set_state(gstreamer::State::Null);
    }
    app.audio_states[i] = AudioState::Muted;

    let (restarted, was_recording) = {
        let mut bridge = app.engine.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let was_recording = bridge.is_recording();
        bridge.note_reconnect();
        // `stop` finalises the current segment; the recording resumes into a
        // fresh segment below rather than silently ending at the outage.
        bridge.stop();
        match bridge.start_from_config(&cam_config) {
            Ok(()) => {
                if was_recording && let Err(e) = bridge.start_recording() {
                    log::warn!("[{}] Could not resume recording: {}", cam_label, e);
                }
                (true, was_recording)
            }
            Err(e) => {
                log::warn!("[{}] Reconnect failed: {}", cam_label, e);
                (false, was_recording)
            }
        }
    };

    if restarted {
        // One rebuild per backoff period, and a fresh connect grace for it.
        app.engine.backoff_states[i].disarm();
        app.engine.connecting_since[i] = Some(Instant::now());
        if prev_audio.is_audible() {
            app.audio_states[i] = prev_audio;
            spawn_audio(app, i, prev_audio.volume_f32());
        }
    } else {
        // Only a genuine failure feeds the exponential backoff; a successful
        // rebuild must not inflate `consecutive_failures` (which would drag the
        // reconnect delay up and, before the cap, could overflow it).
        app.engine.backoff_states[i].record_failure();
    }

    if i < app.sidebar.cameras.len() {
        app.sidebar.cameras[i].status = sidebar::CameraStatus::Reconnecting;
    }
    if restarted && was_recording {
        push_event(
            app,
            i,
            EventType::RecordingStart,
            Some("resumed after reconnect".into()),
        );
    }
}

fn update_selected_metrics(app: &mut App) {
    let Some(idx) = app.sidebar.selected else {
        return;
    };
    if idx >= app.engine.bridges.len() {
        return;
    }

    let bridge = app.engine.bridges[idx]
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let m = bridge.metrics().clone();
    let hints = crate::domain::diagnostics::diagnose(&m);
    app.sidebar.diagnostics = hints;

    let fps = m.snapshot_current_fps().unwrap_or(0.0);
    let si = m.snapshot_stream_info();
    let pkt = m.snapshot_packet_stats();
    let (avg_luma, luma_stddev) = m.snapshot_luma();

    app.sidebar.selected_metrics = Some(super::sidebar::MetricsSnapshot {
        fps,
        latency_ms: m.snapshot_actual_latency_ms(),
        jitter_ms: m.snapshot_jitter_ms(),
        decode_time_ms: m.snapshot_decode_time_ms(),
        packet_stats: pkt,
        dropped: m.dropped_frames.load(Ordering::Relaxed),
        decode_errors: m.decode_errors.load(Ordering::Relaxed),
        reconnects: m.reconnect_count.load(Ordering::Relaxed),
        last_reconnect_ms: {
            let dur = m.last_reconnect_duration_ms.load(Ordering::Relaxed);
            if dur > 0 { Some(dur) } else { None }
        },
        frames: m.frame_count.load(Ordering::Relaxed),
        bytes: m.bytes_counter.load(Ordering::Relaxed),
        uptime_secs: bridge.uptime_secs(),
        codec: si.codec.clone(),
        decoder: si.decoder.clone(),
        decoder_hw: si.decoder_hw,
        stream_quality: app
            .engine
            .camera_configs
            .get(idx)
            .filter(|c| c.sub_url.is_some())
            .and_then(|_| app.engine.stream_quality.get(idx).map(|q| q.label())),
        width: si.width,
        height: si.height,
        framerate_num: si.framerate_num,
        framerate_den: si.framerate_den,
        avg_luma,
        luma_stddev,
        static_secs: m.snapshot_static_secs(now_unix_secs()),
        last_error: m.snapshot_last_error(),
        is_live: m.is_live.load(Ordering::Relaxed),
        is_recording: bridge.is_recording(),
        recording_elapsed_secs: bridge.recording_elapsed_secs(),
        audio_level: app
            .audio_level_states
            .get(idx)
            .map(|l| l.level())
            .unwrap_or(0.0),
        vu_peak: app.vu_peaks.get(idx).copied().unwrap_or(0.0),
        fps_history: app.fps_history.get(idx).cloned().unwrap_or_default(),
    });
}

/// Map the configured 1-100 quality onto the PNG encoder's compression
/// effort. PNG is lossless, so this trades encode time against file size.
fn png_compression(quality: u8) -> image::codecs::png::CompressionType {
    use image::codecs::png::CompressionType;
    match quality {
        0..=49 => CompressionType::Fast,
        50..=89 => CompressionType::Default,
        _ => CompressionType::Best,
    }
}

/// Everything `encode_and_write_snapshot` needs, pulled off `App` on the UI
/// thread so the encode itself can run on a background task.
struct SnapshotJob {
    /// Reference-counted RGBA pixels shared with the appsink — no copy.
    rgba: iced::advanced::image::Bytes,
    width: u32,
    height: u32,
    dir: std::path::PathBuf,
    quality: u8,
    timestamp: u64,
    sequence: u32,
}

/// Grab the latest frame for `idx` (fast, holds the bridge lock briefly).
fn grab_snapshot_job(
    app: &App,
    idx: usize,
    timestamp: u64,
    sequence: u32,
) -> Result<SnapshotJob, String> {
    let bridge = app.engine.bridges[idx]
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let (rgba, width, height) = bridge.capture_frame().ok_or("no frame available yet")?;
    drop(bridge);

    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(format!(
            "frame buffer is {} bytes but {}x{} needs {}",
            rgba.len(),
            width,
            height,
            expected
        ));
    }

    Ok(SnapshotJob {
        rgba,
        width,
        height,
        dir: app.snapshot_config.dir.clone(),
        quality: app.snapshot_config.quality,
        timestamp,
        sequence,
    })
}

/// Encode the frame to PNG and write it. The slow part (compression can take
/// hundreds of ms at high quality); runs on a background task, never the iced
/// update loop.
fn encode_and_write_snapshot(job: SnapshotJob) -> Result<std::path::PathBuf, String> {
    use image::ImageEncoder;

    let (pixels, _) = job.rgba.as_chunks::<4>();
    let rgb_pixels: Vec<u8> = pixels.iter().flat_map(|p| [p[0], p[1], p[2]]).collect();

    std::fs::create_dir_all(&job.dir)
        .map_err(|e| format!("cannot create {}: {e}", job.dir.display()))?;

    let path = job.dir.join(crate::domain::snapshot::generate_filename(
        job.timestamp,
        job.sequence,
    ));

    let mut png_bytes: Vec<u8> = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png_bytes,
        png_compression(job.quality),
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        &rgb_pixels,
        job.width,
        job.height,
        image::ExtendedColorType::Rgb8,
    )
    .map_err(|e| format!("PNG encode failed: {e}"))?;

    // Write to a temporary file first so a reader never sees a partial PNG.
    let tmp_path = path.with_extension("png.tmp");
    std::fs::write(&tmp_path, &png_bytes).map_err(|e| format!("write failed: {e}"))?;
    std::fs::rename(&tmp_path, &path).map_err(|e| format!("rename failed: {e}"))?;

    Ok(path)
}

/// Build the background task that encodes and writes one snapshot frame.
fn snapshot_task(job: SnapshotJob, camera_idx: usize) -> Task<Message> {
    let sequence = job.sequence;
    Task::perform(
        async move { encode_and_write_snapshot(job) },
        move |result| Message::SnapshotSaved {
            camera_idx,
            sequence,
            result,
        },
    )
}

fn update_snapshot(app: &mut App) -> Task<Message> {
    let Some(idx) = app.sidebar.selected else {
        toast(app, "Selecione uma câmera primeiro");
        return Task::none();
    };
    if idx >= app.engine.bridges.len() {
        return Task::none();
    }

    let timestamp = now_unix_secs();
    let burst_count = app.snapshot_config.burst_count.max(1);

    let job = match grab_snapshot_job(app, idx, timestamp, 1) {
        Ok(job) => job,
        Err(e) => {
            log::warn!("Falha no snapshot: {e}");
            toast(app, format!("Falha no snapshot: {e}"));
            return Task::none();
        }
    };

    if burst_count > 1 {
        toast(app, format!("Rajada 1/{burst_count}"));
        app.pending_burst = Some(PendingBurst {
            camera_idx: idx,
            timestamp,
            next_seq: 2,
            remaining: burst_count - 1,
            next_at: Instant::now() + Duration::from_millis(BURST_INTERVAL_MS),
        });
    }

    // The frame is captured; encode + write it off the update loop.
    snapshot_task(job, idx)
}

/// Capture the next frame of an in-flight burst. Driven by the frame tick,
/// whose period equals `BURST_INTERVAL_MS`. Returns the encode task for that
/// frame (or `Task::none()` when it is not yet time for the next frame).
fn advance_burst(app: &mut App) -> Task<Message> {
    let Some(burst) = app.pending_burst.as_ref() else {
        return Task::none();
    };
    if Instant::now() < burst.next_at {
        return Task::none();
    }

    let (idx, timestamp, seq) = (burst.camera_idx, burst.timestamp, burst.next_seq);
    let total = seq + burst.remaining - 1;

    let task = match grab_snapshot_job(app, idx, timestamp, seq) {
        Ok(job) => snapshot_task(job, idx),
        Err(e) => {
            log::warn!("Burst frame {seq} failed: {e}");
            Task::none()
        }
    };

    let Some(burst) = app.pending_burst.as_mut() else {
        return task;
    };
    burst.remaining -= 1;
    burst.next_seq += 1;
    burst.next_at = Instant::now() + Duration::from_millis(BURST_INTERVAL_MS);

    if burst.remaining == 0 {
        app.pending_burst = None;
        toast(app, format!("Rajada concluída: {total} quadros"));
    }

    task
}

fn update_recording(app: &mut App) -> Task<Message> {
    let Some(idx) = app.sidebar.selected else {
        toast(app, "Selecione uma câmera primeiro");
        return Task::none();
    };
    if idx >= app.engine.bridges.len() {
        return Task::none();
    }

    match toggle_camera_recording(app, idx) {
        Ok(true) => toast(app, "Gravação iniciada"),
        Ok(false) => toast(app, "Gravação parada"),
        Err(e) => {
            log::error!("Recording toggle failed: {e}");
            toast(app, format!("Falha na gravação: {e}"));
        }
    }
    Task::none()
}

/// Flip a camera's recording state and mirror it into the sidebar and the
/// timeline. Shared by the `r` key and the motion trigger.
fn toggle_camera_recording(app: &mut App, idx: usize) -> Result<bool, String> {
    // A recording is taken from the decoded frames, so starting one on the
    // sub-stream would save a low-resolution file. Move to the main stream
    // first; `wanted_quality` then leaves it alone until the recording stops.
    let starting = !app.engine.bridges[idx]
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_recording();
    if starting && app.engine.stream_quality.get(idx) == Some(&StreamQuality::Sub) {
        restart_stream(app, idx, StreamQuality::Main);
    }
    let is_recording = {
        let mut bridge = app.engine.bridges[idx]
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        bridge.toggle_recording()?
    };
    app.is_recording = is_recording;
    if idx < app.sidebar.cameras.len() {
        app.sidebar.cameras[idx].status = if is_recording {
            sidebar::CameraStatus::Recording
        } else {
            sidebar::CameraStatus::Live
        };
    }
    let kind = if is_recording {
        EventType::RecordingStart
    } else {
        EventType::RecordingStop
    };
    push_event(app, idx, kind, None);
    Ok(is_recording)
}

/// `[recording] on_motion`: start recording on motion, stop after the
/// post-roll of quiet. Runs right after each motion sample.
fn drive_motion_recording(app: &mut App, i: usize) {
    if !app.engine.motion_recording {
        return;
    }
    let is_recording = app.engine.bridges[i]
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_recording();
    if !is_recording {
        // Stopped by the user (or a reconnect): the trigger no longer owns it.
        app.engine.auto_recording[i] = false;
    }
    if app.engine.motion_active[i] {
        app.engine.last_motion_at[i] = Some(Instant::now());
    }
    let quiet = app.engine.last_motion_at[i].map_or(u64::MAX, |t| t.elapsed().as_secs());
    let action = crate::domain::recording::motion_recording_action(
        is_recording,
        app.engine.auto_recording[i],
        app.engine.motion_active[i],
        quiet,
        app.engine.motion_post_roll_secs,
    );
    match action {
        crate::domain::recording::MotionRecAction::None => {}
        crate::domain::recording::MotionRecAction::Start => match toggle_camera_recording(app, i) {
            Ok(_) => app.engine.auto_recording[i] = true,
            Err(e) => log::warn!("Motion recording could not start on camera {i}: {e}"),
        },
        crate::domain::recording::MotionRecAction::Stop => {
            if let Err(e) = toggle_camera_recording(app, i) {
                log::warn!("Motion recording could not stop on camera {i}: {e}");
            }
            app.engine.auto_recording[i] = false;
        }
    }
}

/// Push a volume value to a running audio pipeline, if there is one.
fn apply_volume(app: &App, idx: usize, volume: f64) -> bool {
    if let Some(pipeline) = app.audio_pipelines[idx].borrow().as_ref() {
        if let Some(vol_elem) = pipeline.by_name("audio_volume") {
            vol_elem.set_property("volume", volume);
        }
        true
    } else {
        false
    }
}

/// Build and start an audio pipeline for a camera at the given volume.
fn spawn_audio(app: &mut App, idx: usize, volume: f32) {
    let url = app.audio_urls[idx].clone();
    if url.is_empty() {
        return;
    }
    let Some(pipeline) = build_audio_pipeline_for_url(&url, volume) else {
        log::warn!("Could not build audio pipeline for camera {idx}");
        return;
    };
    // The VU level is read by polling the bus each tick (see `update_frame`);
    // a `bus.add_watch` would never fire without a GLib main loop.
    let _ = pipeline.set_state(gstreamer::State::Playing);
    *app.audio_pipelines[idx].borrow_mut() = Some(pipeline);
}

fn update_audio(app: &mut App) -> Task<Message> {
    if !app.audio_config.enabled {
        toast(app, "Áudio desativado no config.toml");
        return Task::none();
    }
    let Some(idx) = app.sidebar.selected else {
        toast(app, "Selecione uma câmera primeiro");
        return Task::none();
    };
    if idx >= app.audio_states.len() {
        return Task::none();
    }

    let old_state = app.audio_states[idx];
    let (new_state, volume_x1000) = if old_state.is_muted() {
        // Per-camera `audio_volume` wins; otherwise fall back to the global
        // `[audio] volume` rather than a hardcoded default.
        let vol_x1000 = app
            .engine
            .camera_configs
            .get(idx)
            .and_then(|c| c.audio_volume)
            .unwrap_or(app.audio_config.volume)
            .clamp(0.0, 1.0);
        let vol_x1000 = ((vol_x1000 * 1000.0) as u32).max(1);
        (
            AudioState::Live {
                volume_x1000: vol_x1000,
            },
            vol_x1000,
        )
    } else {
        old_state.toggle()
    };
    app.audio_states[idx] = new_state;

    if !apply_volume(app, idx, volume_x1000 as f64 / 1000.0) && new_state.is_audible() {
        spawn_audio(app, idx, new_state.volume_f32());
    }
    Task::none()
}

fn update_volume(app: &mut App, up: bool) -> Task<Message> {
    if !app.audio_config.enabled {
        toast(app, "Áudio desativado no config.toml");
        return Task::none();
    }
    let Some(idx) = app.sidebar.selected else {
        return Task::none();
    };
    if idx >= app.audio_states.len() {
        return Task::none();
    }

    let current_vol = app.audio_states[idx].volume_x1000();
    let new_vol_x1000 = if up {
        (current_vol + 50).min(1000)
    } else {
        current_vol.saturating_sub(50)
    };
    let new_state = if new_vol_x1000 == 0 {
        AudioState::Muted
    } else {
        AudioState::Live {
            volume_x1000: new_vol_x1000,
        }
    };
    app.audio_states[idx] = new_state;

    if !apply_volume(app, idx, new_vol_x1000 as f64 / 1000.0) && up && new_vol_x1000 > 0 {
        spawn_audio(app, idx, new_vol_x1000 as f32 / 1000.0);
    }
    Task::none()
}

fn update_sidebar(app: &mut App, msg: super::sidebar::Message) -> Task<Message> {
    match msg {
        super::sidebar::Message::TabClicked(view) => {
            app.sidebar.active_view = view;
            // `BlurSearch` both clears the flag and issues the unfocus op, so
            // the flag and the real `text_input` focus can't drift apart.
            return update(app, Message::BlurSearch);
        }
        super::sidebar::Message::EventClicked(idx) => {
            return update(app, Message::EventClicked(idx));
        }
        super::sidebar::Message::CameraClicked(idx) => {
            app.sidebar.selected = Some(idx);
            app.flex_main_idx = idx;
            app.sidebar.active_view = sidebar::SidebarView::Info;
            return update(app, Message::BlurSearch);
        }
        super::sidebar::Message::SearchChanged(q) => {
            // Receiving input proves the box owns the keyboard.
            app.search_focused = true;
            app.sidebar.search_query = q;
        }
        super::sidebar::Message::SearchSubmitted => {
            return update(app, Message::BlurSearch);
        }
        super::sidebar::Message::GroupSelected(group) => {
            if group.is_none_or(|g| g < app.sidebar.groups.len()) {
                app.sidebar.active_group = group;
                app.current_page = 0;
                sync_active_streams(app);
                persist_view(app);
            }
        }
        super::sidebar::Message::HoverRow(row) => {
            app.sidebar.hover_row = row;
        }
        super::sidebar::Message::ShowRowMenu(idx) => {
            app.sidebar.selected = Some(idx);
            app.context_menu = Some(super::state::ContextMenu {
                camera_idx: idx,
                anchor: app.pointer_pos,
            });
        }
        super::sidebar::Message::ToggleInfoAdvanced => {
            app.sidebar.info_advanced = !app.sidebar.info_advanced;
        }
        super::sidebar::Message::CameraMovedUp(idx)
        | super::sidebar::Message::CameraMovedDown(idx) => {
            if matches!(msg, super::sidebar::Message::CameraMovedUp(_)) {
                view::move_up(&mut app.view.order, idx);
            } else {
                view::move_down(&mut app.view.order, idx);
            }
            app.sidebar.order = app.view.order.clone();
            clamp_current_page(app);
            sync_active_streams(app);
            persist_view(app);
        }
        super::sidebar::Message::CameraToggled(idx, enabled) => {
            if idx < app.engine.camera_enabled.len() {
                app.engine.camera_enabled[idx] = enabled;
                app.sidebar.cameras[idx].enabled = enabled;

                if !enabled {
                    app.engine.bridges[idx]
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .stop();
                    if let Some(pipeline) = app.audio_pipelines[idx].borrow_mut().take() {
                        let _ = pipeline.set_state(gstreamer::State::Null);
                    }
                    app.audio_states[idx] = AudioState::Muted;
                    app.engine.active_stream[idx] = false;
                    app.engine.connecting_since[idx] = None;
                    app.engine.start_queue.retain(|&q| q != idx);
                    app.sidebar.cameras[idx].status = sidebar::CameraStatus::Disabled;
                    app.engine.reconnect_states[idx].reset();
                    app.engine.backoff_states[idx] = super::state::BackoffState::new();
                } else {
                    // Re-enabled: leave the actual (re)start to
                    // `sync_active_streams`, which only spins it up if the
                    // camera is on the visible page.
                    app.engine.active_stream[idx] = false;
                    app.engine.backoff_states[idx] = super::state::BackoffState::new();
                    app.engine.reconnect_states[idx].reset();
                    if idx < app.sidebar.cameras.len() {
                        app.sidebar.cameras[idx].status = sidebar::CameraStatus::Connecting;
                    }
                }
                sync_active_streams(app);
            }
        }
    }
    Task::none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_closes_on_escape_or_question_mark_only() {
        use iced::keyboard::Key;
        use iced::keyboard::key::Named;
        assert!(is_help_dismiss(&Key::Named(Named::Escape)));
        assert!(is_help_dismiss(&Key::Character("?".into())));
        for other in ["f", "s", "r", "h", "1"] {
            assert!(!is_help_dismiss(&Key::Character(other.into())));
        }
        assert!(!is_help_dismiss(&Key::Named(Named::Enter)));
    }

    #[test]
    fn png_quality_maps_to_compression_effort() {
        use image::codecs::png::CompressionType;
        assert!(matches!(png_compression(10), CompressionType::Fast));
        assert!(matches!(png_compression(85), CompressionType::Default));
        assert!(matches!(png_compression(92), CompressionType::Best));
    }

    #[test]
    fn burst_interval_matches_the_frame_tick() {
        // `advance_burst` is driven by the frame tick, so a burst would be
        // paced wrong if these two ever diverged.
        assert_eq!(BURST_INTERVAL_MS, super::super::subscription::TICK_MS);
    }
}
