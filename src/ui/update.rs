use crate::i18n::{t, tf};
use iced::Task;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use gstreamer::prelude::*;

use crate::domain::audio::AudioState;
use crate::domain::multi_stream::StreamQuality;
use crate::domain::snapshot::BURST_INTERVAL_MS;
use crate::domain::timeline::{EventType, TimelineEvent};
use crate::domain::view;
use crate::infrastructure::audio::{build_audio_pipeline_for_url, poll_level_bus};
use crate::infrastructure::view_state::ViewStateFile;

use super::app::PendingBurst;
use super::bridge::now_unix_secs;
use super::daemon::{Effect, PendingRequest};
use super::message::LayoutMode;
use super::recordings::RecMsg;
use super::sidebar;
use super::state::{Toast, VU_PEAK_DECAY_MS};
use super::{App, Message};

/// Focus target used to blur the search box: focusing an id that no widget
/// owns makes iced unfocus everything.
const BLUR_TARGET: &str = "__rrv_blur__";

pub fn update(app: &mut App, message: Message) -> Task<Message> {
    let task = update_inner(app, message);
    drain_engine_events(app);
    sync_status_rows(app);
    task
}

/// The engine owns each camera's status; the sidebar rows (read by the views)
/// mirror it. One place, after every update, keeps them from drifting.
fn sync_status_rows(app: &mut App) {
    for (row, status) in app.sidebar.cameras.iter_mut().zip(&app.engine.status) {
        if row.status != *status {
            row.status = status.clone();
        }
    }
    // Com um daemon, quem grava é ele: o REC vem do que ele diz, nunca do que
    // esta janela decodifica. A ligação é pelo nome da câmera.
    if app.daemon.is_connected() {
        for row in app.sidebar.cameras.iter_mut() {
            let Some(info) = app.daemon.info_for(&row.name) else {
                continue;
            };
            row.is_recording = info.recording;
            if info.recording && row.status == sidebar::CameraStatus::Live {
                row.status = sidebar::CameraStatus::Recording;
            } else if !info.recording && row.status == sidebar::CameraStatus::Recording {
                row.status = sidebar::CameraStatus::Live;
            }
        }
        app.is_recording = app
            .sidebar
            .selected
            .and_then(|i| app.sidebar.cameras.get(i))
            .is_some_and(|c| c.is_recording);
    }
}

fn update_inner(app: &mut App, message: Message) -> Task<Message> {
    match message {
        Message::FrameUpdate => update_frame(app),
        Message::Recordings(m) => super::recordings::update(app, m),
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
                saving: false,
            });
            toast(
                app,
                t("Zonas: clique para marcar os pontos · Enter conclui · Esc sai"),
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
                if app.daemon.is_daemon_mode() {
                    send_zones(app, idx, Vec::new());
                    return Task::none();
                }
                if let Some(cfg) = app.engine.zones.get_mut(idx) {
                    cfg.zones.clear();
                }
                persist_zones(app, idx);
                toast(app, t("Zonas removidas: o quadro inteiro conta"));
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
        Message::OpenAddCamera => {
            let (wizard, effect) = super::add_camera::AddCamera::open();
            app.add_camera = Some(wizard);
            add_camera_effect(app, effect)
        }
        Message::AddCamera(msg) => {
            let Some(wizard) = app.add_camera.as_mut() else {
                return Task::none();
            };
            let effect = wizard.apply(msg);
            add_camera_effect(app, effect)
        }
        Message::LanguageChanged(lang) => {
            crate::i18n::set(lang);
            persist_view(app);
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
                    tf("Carrossel ligado ({}s)", &[&app.view.rotate_secs])
                } else {
                    t("Carrossel desligado").to_string()
                },
            );
            Task::none()
        }
        Message::RotateIntervalStep(up) => {
            app.view.rotate_secs = view::step_rotate_secs(app.view.rotate_secs, up);
            app.rotate_last_advance = Instant::now();
            persist_view(app);
            toast(app, tf("Carrossel: {}s", &[&app.view.rotate_secs]));
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
                            message: tf("Snapshot: {} · clique para abrir a pasta", &[&name]),
                            shown_at: Instant::now(),
                            open_dir: path.parent().map(std::path::Path::to_path_buf),
                        });
                    }
                }
                Err(e) => {
                    log::warn!("Falha no snapshot: {e}");
                    toast(app, tf("Falha no snapshot: {}", &[&e]));
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
            iced::window::latest().and_then(move |id| iced::window::set_mode(id, new_mode))
        }
        Message::ExitFullscreen => {
            if app.is_fullscreen {
                app.is_fullscreen = false;
                if app.focus == super::app::ViewFocus::Immersive {
                    app.focus = super::app::ViewFocus::Normal;
                }
                return iced::window::latest()
                    .and_then(|id| iced::window::set_mode(id, iced::window::Mode::Windowed));
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
            app.daemon.menu_open = false;
            update(app, *inner)
        }
        Message::QuitRequested => {
            match super::daemon::Modal::for_quit(local_recording_count(app)) {
                Some(modal) => {
                    app.modal = Some(modal);
                    Task::none()
                }
                None => update(app, Message::Quit),
            }
        }
        Message::ToggleDaemonMenu => {
            app.daemon.menu_open = !app.daemon.menu_open;
            app.show_overflow_menu = false;
            app.context_menu = None;
            Task::none()
        }
        Message::DaemonReconnect => {
            if let Some(link) = &app.link {
                link.reconnect_now();
                toast(app, t("Reconectando ao daemon…"));
            }
            Task::none()
        }
        Message::AskUseLocalEngine => {
            app.modal = Some(super::daemon::Modal::UseLocalEngine);
            Task::none()
        }
        Message::CopyDaemonStartCommand => {
            toast(app, t("Comando copiado: docker compose up -d"));
            iced::clipboard::write("docker compose up -d".to_string())
        }
        Message::CopyDaemonSocket => {
            toast(app, t("Caminho do socket copiado"));
            iced::clipboard::write(app.daemon.socket.display().to_string())
        }
        Message::ModalCancel => {
            app.modal = None;
            Task::none()
        }
        Message::ModalConfirm => match app.modal.take() {
            Some(super::daemon::Modal::UseLocalEngine) => {
                use_local_engine(app);
                Task::none()
            }
            Some(super::daemon::Modal::Quit { .. }) => update(app, Message::Quit),
            None => Task::none(),
        },
        Message::Quit => {
            shutdown(app);
            iced::window::latest().and_then(iced::window::close)
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
            iced::widget::operation::focus(sidebar::cameras::search_input_id())
        }
        Message::BlurSearch => {
            app.search_focused = false;
            iced::widget::operation::focus(iced::widget::Id::new(BLUR_TARGET))
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
/// Keys while the recordings view is open.
fn recordings_key(app: &mut App, key: iced::keyboard::Key<&str>) -> Task<Message> {
    use iced::keyboard::Key;
    use iced::keyboard::key::Named;
    let msg = match key {
        Key::Named(Named::Escape) => RecMsg::Close,
        Key::Named(Named::Space) => RecMsg::PlayPause,
        Key::Named(Named::ArrowLeft) => RecMsg::Skip(-10_000),
        Key::Named(Named::ArrowRight) => RecMsg::Skip(10_000),
        Key::Character("l") => RecMsg::Live,
        Key::Character("i") => RecMsg::MarkIn,
        Key::Character("o") => RecMsg::MarkOut,
        Key::Character("e") => RecMsg::Export,
        Key::Character("p") => RecMsg::ToggleProtect,
        Key::Character(",") => RecMsg::Rate(0.5),
        Key::Character(".") => RecMsg::Rate(2.0),
        Key::Character("-") => RecMsg::Zoom {
            factor: 1.25,
            anchor: 1.0,
        },
        Key::Character("+") | Key::Character("=") => RecMsg::Zoom {
            factor: 0.8,
            anchor: 1.0,
        },
        _ => return Task::none(),
    };
    update(app, Message::Recordings(msg))
}

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
            Key::Character("q") => update(app, Message::QuitRequested),
            _ => Task::none(),
        };
    }

    // O assistente "Adicionar câmera" tem campos de texto: só o Esc age fora deles.
    if app.add_camera.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Escape) => {
                update(app, Message::AddCamera(super::add_camera::AddMsg::Close))
            }
            _ => Task::none(),
        };
    }

    // A confirmation is modal for the keyboard: Enter accepts, Esc declines,
    // nothing else may fire on the UI behind the scrim.
    if app.modal.is_some() {
        return match key.as_ref() {
            Key::Named(Named::Enter) => update(app, Message::ModalConfirm),
            Key::Named(Named::Escape) => update(app, Message::ModalCancel),
            _ => Task::none(),
        };
    }
    if app.daemon.menu_open && matches!(key.as_ref(), Key::Named(Named::Escape)) {
        return update(app, Message::ToggleDaemonMenu);
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

    // The recordings view owns the keyboard while it is open.
    if app.recordings.is_some() {
        return recordings_key(app, key.as_ref());
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
        Key::Character("t") => update(app, Message::Recordings(RecMsg::Open)),
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
    if edit.saving {
        return; // aguardando o daemon
    }
    if edit.temp_vertices.len() < 3 {
        toast(app, t("Uma zona precisa de pelo menos 3 pontos"));
        return;
    }
    if crate::domain::zones::polygon_area(&edit.temp_vertices) < MIN_ZONE_AREA {
        toast(
            app,
            t("Zona sem área: os pontos estão alinhados ou repetidos"),
        );
        return;
    }
    let idx = edit.camera_idx;

    if app.daemon.is_daemon_mode() {
        // O daemon é dono das zonas: a janela só as aplica quando ele confirma,
        // e o editor segue aberto com o desenho se ele recusar.
        let vertices = edit.temp_vertices.clone();
        let mut zones = app
            .engine
            .zones
            .get(idx)
            .map(|c| c.zones.clone())
            .unwrap_or_default();
        let name = tf("Zona {}", &[&(zones.len() + 1)]);
        zones.push(crate::domain::zones::MotionZone::new(name, vertices));
        send_zones(app, idx, zones);
        return;
    }

    let vertices = std::mem::take(&mut edit.temp_vertices);
    if let Some(cfg) = app.engine.zones.get_mut(idx) {
        let name = tf("Zona {}", &[&(cfg.zones.len() + 1)]);
        cfg.zones
            .push(crate::domain::zones::MotionZone::new(name, vertices));
    }
    persist_zones(app, idx);
    toast(app, t("Zona salva"));
}

/// Ask the daemon to store `zones` for camera `idx` (a camera of this window).
fn send_zones(app: &mut App, idx: usize, zones: Vec<crate::domain::zones::MotionZone>) {
    let Some(name) = app.sidebar.cameras.get(idx).map(|c| c.name.clone()) else {
        return;
    };
    let Some(index) = daemon_index(app, &name) else {
        return;
    };
    let wire = zones
        .iter()
        .map(crate::domain::zones::MotionZoneFile::from_zone)
        .collect();
    let ok = send_to_daemon(
        app,
        crate::ipc::protocol::Request::SetZones {
            camera: index,
            zones: wire,
        },
        PendingRequest::SetZones { camera: idx, zones },
    );
    if ok && let Some(edit) = app.zone_edit.as_mut() {
        edit.saving = true;
    }
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

/// Record an event: the engine applies its notification policy and the host
/// puts it on the timeline (see [`drain_engine_events`]).
fn push_event(app: &mut App, camera_idx: usize, kind: EventType, description: Option<String>) {
    app.engine.emit(camera_idx, kind, description);
    drain_engine_events(app);
}

/// The host side of the engine's events: put them on the timeline and deliver
/// the desktop notification the engine's policy asked for.
fn drain_engine_events(app: &mut App) {
    for ev in app.engine.take_events() {
        if let Some((title, body)) = &ev.notification {
            crate::infrastructure::notify::send(title, body);
        }
        let mut event = TimelineEvent::new(now_unix_secs(), ev.camera, ev.kind);
        if let Some(d) = ev.detail {
            event = event.with_description(d);
        }
        app.sidebar.timeline.push(event);
    }
}

/// Recordings this window is making itself (zero with a daemon: it records).
fn local_recording_count(app: &App) -> usize {
    if app.daemon.is_daemon_mode() {
        return 0;
    }
    app.engine
        .bridges
        .iter()
        .filter(|b| b.lock().unwrap_or_else(|e| e.into_inner()).is_recording())
        .count()
}

/// "Usar o motor local": this window takes over recording and detection. Only
/// after the user confirmed (the daemon may still be recording).
fn use_local_engine(app: &mut App) {
    let socket = app.daemon.socket.clone();
    // Closing the connection joins its thread; do it off the UI thread.
    if let Some(link) = app.link.take() {
        std::thread::spawn(move || drop(link));
    }
    app.pending.clear();
    app.daemon = super::daemon::DaemonState::embedded(socket);
    app.engine.set_display_only(false);
    sync_active_streams(app);
    toast(
        app,
        t("Motor local ativado: esta janela agora grava e detecta"),
    );
}

// ───────────────────────────── the rrv-daemon ─────────────────────────────

/// Drain what the connection thread reported since the last tick.
fn poll_daemon(app: &mut App) {
    let Some(link) = app.link.as_ref() else {
        return;
    };
    let events: Vec<_> = link.events.try_iter().collect();
    for event in events {
        match app.daemon.apply(event) {
            Effect::None => {}
            Effect::Toast(text) => toast(app, text),
            Effect::Event(wire) => daemon_event(app, wire),
            Effect::Reply { token, result } => handle_reply(app, token, result),
        }
    }
    if app.open_recordings_on_connect && app.daemon.is_connected() {
        app.open_recordings_on_connect = false;
        let _ = super::recordings::update(app, RecMsg::Open);
    }
}

/// An event from the daemon: onto the timeline and, if its policy asked for
/// one, a desktop notification (only while this window is open).
fn daemon_event(app: &mut App, wire: crate::ipc::protocol::WireEvent) {
    // O daemon decidiu o aviso, mas o desktop é desta janela: só aparece se a
    // configuração dela pediu (`[notifications] enabled`).
    if app.engine.configured.notify_enabled
        && let Some((title, body)) = &wire.notification
    {
        crate::infrastructure::notify::send(title, body);
    }
    // A system event, not a camera's: the disk is nearly full.
    if wire.kind == EventType::DiskLow {
        if let Some(d) = &wire.detail
            && wire.notification.is_some()
        {
            toast(app, tf("Disco das gravações quase cheio: {}", &[&d]));
        }
        return;
    }
    let Some(idx) = app.sidebar.cameras.iter().position(|c| c.name == wire.name) else {
        return; // a camera this window does not have
    };
    let mut event = TimelineEvent::new(wire.unix_secs, idx, wire.kind);
    if let Some(d) = wire.detail {
        event = event.with_description(d);
    }
    app.sidebar.timeline.push(event);
}

/// Send a request to the daemon, remembering what to do with the reply. Returns
/// `false` (with a toast) when there is no live connection: nothing is ever
/// applied locally on faith.
pub(super) fn send_to_daemon(
    app: &mut App,
    request: crate::ipc::protocol::Request,
    pending: PendingRequest,
) -> bool {
    if !app.daemon.is_connected() {
        toast(
            app,
            t("Sem conexão com o daemon: tente de novo quando ele voltar"),
        );
        return false;
    }
    let Some(link) = app.link.as_ref() else {
        return false;
    };
    let token = app.next_token;
    app.next_token += 1;
    app.pending.insert(token, pending);
    link.request(token, request);
    true
}

/// The daemon's index for a camera of this window, found by name.
fn daemon_index(app: &mut App, name: &str) -> Option<usize> {
    let found = app.daemon.info_for(name).map(|c| c.index);
    if found.is_none() {
        toast(app, tf("O daemon não conhece a câmera '{}'", &[&name]));
    }
    found
}

/// What came back for a request this window made.
fn handle_reply(app: &mut App, token: u64, result: Result<crate::ipc::protocol::Response, String>) {
    use crate::ipc::protocol::Response;
    let Some(pending) = app.pending.remove(&token) else {
        return;
    };
    let failure = match &result {
        Ok(Response::Error { message }) | Err(message) => Some(message.clone()),
        Ok(_) => None,
    };
    match (pending, failure) {
        (PendingRequest::History, _) => super::recordings::on_history(app, result),
        (PendingRequest::Export, _) => super::recordings::on_exported(app, result),
        (
            PendingRequest::Protect {
                segment_id,
                protected,
            },
            _,
        ) => super::recordings::on_protected(app, segment_id, protected, result),
        (PendingRequest::ToggleRecording { .. }, None) => {
            if let Ok(Response::Recording { recording, .. }) = result {
                toast(
                    app,
                    if recording {
                        t("Gravação iniciada no daemon")
                    } else {
                        t("Gravação parada no daemon")
                    },
                );
            }
        }
        (PendingRequest::SetEnabled { .. }, None) => {}
        (PendingRequest::SetZones { camera, zones }, None) => {
            if let Some(cfg) = app.engine.zones.get_mut(camera) {
                cfg.zones = zones;
            }
            if let Some(edit) = app.zone_edit.as_mut() {
                edit.temp_vertices.clear();
                edit.saving = false;
            }
            toast(app, t("Zona salva no daemon"));
        }
        (PendingRequest::SetZones { .. }, Some(why)) => {
            // O editor continua aberto com o desenho intacto.
            if let Some(edit) = app.zone_edit.as_mut() {
                edit.saving = false;
            }
            toast(app, tf("Não foi possível salvar a zona: {}", &[&why]));
        }
        (PendingRequest::ToggleRecording { .. }, Some(why)) => {
            toast(app, tf("Falha na gravação: {}", &[&why]));
        }
        (PendingRequest::SetEnabled { camera }, Some(why)) => {
            toast(app, tf("O daemon não aplicou '{}': {}", &[&camera, &why]));
        }
    }
}

pub(super) fn toast(app: &mut App, message: impl Into<String>) {
    app.toasts.push(Toast {
        message: message.into(),
        shown_at: Instant::now(),
        open_dir: None,
    });
}

/// A toast that opens `dir` in the file manager when clicked.
pub(super) fn toast_open_dir(
    app: &mut App,
    message: impl Into<String>,
    dir: Option<std::path::PathBuf>,
) {
    app.toasts.push(Toast {
        message: message.into(),
        shown_at: Instant::now(),
        open_dir: dir,
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
    let enabled_all = app.engine.enabled_cameras();
    // Engine policy: everything keeps decoding when `pause_hidden` is off or
    // something reacts to motion. Only otherwise does the view decide.
    if app.engine.must_run_everything() {
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
    app.engine.restart_stream(i, quality);
}

/// The stream camera `i` should be on given what is on screen.
fn wanted_quality(app: &App, i: usize) -> StreamQuality {
    app.engine.wanted_quality(i, is_large_view(app, i))
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
    app.engine.pause_stream(i);
    // Audio is still the UI's: silence it along with the video.
    if let Some(pipeline) = app.audio_pipelines[i].borrow_mut().take() {
        let _ = pipeline.set_state(gstreamer::State::Null);
    }
    app.audio_states[i] = AudioState::Muted;
}

/// Reconcile running pipelines with [`desired_active_cameras`]. Cameras that
/// should run but don't are queued for the staggered drain; cameras that run
/// but shouldn't are stopped immediately.
fn sync_active_streams(app: &mut App) {
    clamp_current_page(app);
    let want = desired_active_cameras(app);

    for i in app.engine.reconcile(&want) {
        pause_stream(app, i);
    }

    // Running cameras whose view changed (grid tile ↔ spotlight) swap stream.
    for i in 0..app.engine.bridges.len() {
        if !app.engine.camera_enabled[i] || !app.engine.active_stream[i] {
            continue;
        }
        let wanted = wanted_quality(app, i);
        if wanted != app.engine.stream_quality[i] {
            log::info!("Camera {i}: switching to the {} stream", wanted.label());
            app.engine.restart_stream(i, wanted);
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
            .has_frame();
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
            && !app.engine.bridges[i]
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .has_frame()
    });
    if let Some(i) = next {
        app.preview_cam = Some((i, Instant::now()));
        start_stream(app, i);
        app.engine.next_start_at = Instant::now() + app.engine.stagger;
    }
}

/// Start at most one queued camera per `stagger`.
fn drain_start_queue(app: &mut App) {
    if app.engine.start_queue.is_empty() {
        return;
    }
    let want = desired_active_cameras(app);
    if let Some(i) = app.engine.pop_next_start(&want) {
        start_stream(app, i);
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
/// Corre `f` numa thread própria e entrega o resultado (`None` se a thread morreu).
fn on_thread<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> impl std::future::Future<Output = Option<T>> {
    let (tx, rx) = iced::futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    async move { rx.await.ok() }
}

/// O que o assistente "Adicionar câmera" pede à janela depois de uma transição.
fn add_camera_effect(app: &mut App, effect: super::add_camera::Effect) -> Task<Message> {
    use super::add_camera::{AddMsg, Effect, Inspected};
    match effect {
        Effect::None => Task::none(),
        Effect::Close => {
            app.add_camera = None;
            Task::none()
        }
        Effect::Copy(snippet) => {
            toast(app, t("Trecho copiado"));
            iced::clipboard::write(snippet)
        }
        Effect::Scan => Task::perform(
            on_thread(|| crate::onvif::discover(std::time::Duration::from_secs(4))),
            |found| Message::AddCamera(AddMsg::Scanned(found.unwrap_or_default())),
        ),
        Effect::Inspect(found, user, password) => Task::perform(
            on_thread(move || {
                let ins = crate::onvif::inspect(&found, &user, &password)?;
                let secret = crate::onvif::secret_name_for(&found, &ins);
                let stored = crate::secrets::keyring::store(&secret, &password).is_ok();
                Ok::<_, String>(Inspected {
                    snippet: crate::onvif::config_snippet(&found, &user, &ins),
                    secret,
                    stored,
                })
            }),
            |r| {
                Message::AddCamera(AddMsg::Inspected(
                    r.unwrap_or_else(|| Err(t("A leitura foi interrompida").to_string())),
                ))
            },
        ),
    }
}

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
        language: Some(crate::i18n::get().code().to_string()),
    };
    crate::infrastructure::view_state::save(&state);
}

fn update_frame(app: &mut App) -> Task<Message> {
    poll_daemon(app);
    super::recordings::tick(app);
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

        let tick = app.engine.tick_camera(i, poll_slow_metrics);

        // The sidebar row is the UI's: sparkline history, mute and REC flags.
        app.fps_history[i].push(tick.fps);
        if app.fps_history[i].len() > 60 {
            app.fps_history[i].remove(0);
        }
        let is_recording = app.engine.bridges[i]
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_recording();
        let is_muted = !app
            .audio_states
            .get(i)
            .map(|s| s.is_audible())
            .unwrap_or(false);
        let row = &mut app.sidebar.cameras[i];
        if let Some(reading) = tick.reading {
            row.apply(reading);
        }
        row.fps_history.push(tick.fps);
        if row.fps_history.len() > 16 {
            row.fps_history.remove(0);
        }
        row.is_muted = is_muted;
        row.is_recording = is_recording;

        if tick.needs_reconnect {
            let cam_label = app.engine.names[i].clone();
            reconnect_camera(app, i, &cam_label);
        }

        if poll_slow_metrics {
            app.engine.tick_motion(i);
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
    // Remember whether audio was playing so it can be resumed after the
    // rebuild instead of forcing the user to press `m` on every reconnect.
    let prev_audio = app.audio_states[i];
    if let Some(pipeline) = app.audio_pipelines[i].borrow_mut().take() {
        let _ = pipeline.set_state(gstreamer::State::Null);
    }
    app.audio_states[i] = AudioState::Muted;

    let Some(outcome) = app.engine.reconnect(i, cam_label) else {
        return;
    };
    if outcome.restarted && prev_audio.is_audible() {
        app.audio_states[i] = prev_audio;
        spawn_audio(app, i, prev_audio.volume_f32());
    }
    if outcome.resumed_recording {
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
    rgba: bytes::Bytes,
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
    .map_err(|e| format!("PNG encode failed: {e}"))?; // i18n-ok: erro de programação

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
        toast(app, t("Selecione uma câmera primeiro"));
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
            toast(app, tf("Falha no snapshot: {}", &[&e]));
            return Task::none();
        }
    };

    if burst_count > 1 {
        toast(app, tf("Rajada 1/{}", &[&burst_count]));
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
        toast(app, tf("Rajada concluída: {} quadros", &[&total]));
    }

    task
}

fn update_recording(app: &mut App) -> Task<Message> {
    let Some(idx) = app.sidebar.selected else {
        toast(app, t("Selecione uma câmera primeiro"));
        return Task::none();
    };
    if idx >= app.engine.bridges.len() {
        return Task::none();
    }

    // Com um daemon, ele grava: a janela pede e só mostra REC quando ele confirma.
    if app.daemon.is_daemon_mode() {
        let name = app.sidebar.cameras[idx].name.clone();
        let already = app
            .pending
            .values()
            .any(|p| matches!(p, PendingRequest::ToggleRecording { camera } if *camera == name));
        if already {
            return Task::none(); // aguardando a resposta anterior
        }
        if let Some(index) = daemon_index(app, &name) {
            send_to_daemon(
                app,
                crate::ipc::protocol::Request::ToggleRecording { camera: index },
                PendingRequest::ToggleRecording { camera: name },
            );
        }
        return Task::none();
    }

    match toggle_camera_recording(app, idx) {
        Ok(true) => toast(app, t("Gravação iniciada")),
        Ok(false) => toast(app, t("Gravação parada")),
        Err(e) => {
            log::error!("Recording toggle failed: {e}");
            toast(app, tf("Falha na gravação: {}", &[&e]));
        }
    }
    Task::none()
}

/// Flip a camera's recording state and mirror it into the sidebar and the
/// timeline. Shared by the `r` key and the motion trigger.
fn toggle_camera_recording(app: &mut App, idx: usize) -> Result<bool, String> {
    let is_recording = app.engine.toggle_recording(idx)?;
    app.is_recording = is_recording;
    Ok(is_recording)
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
        toast(app, t("Áudio desativado no config.toml"));
        return Task::none();
    }
    let Some(idx) = app.sidebar.selected else {
        toast(app, t("Selecione uma câmera primeiro"));
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
        toast(app, t("Áudio desativado no config.toml"));
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
                app.sidebar.cameras[idx].enabled = enabled;
                app.engine.set_camera_enabled(idx, enabled);
                if app.daemon.is_daemon_mode() {
                    let name = app.sidebar.cameras[idx].name.clone();
                    if let Some(index) = daemon_index(app, &name) {
                        send_to_daemon(
                            app,
                            crate::ipc::protocol::Request::SetCameraEnabled {
                                camera: index,
                                enabled,
                            },
                            PendingRequest::SetEnabled { camera: name },
                        );
                    }
                }
                if !enabled {
                    // Audio is still the UI's: silence it with the video.
                    if let Some(pipeline) = app.audio_pipelines[idx].borrow_mut().take() {
                        let _ = pipeline.set_state(gstreamer::State::Null);
                    }
                    app.audio_states[idx] = AudioState::Muted;
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

    // ---- a janela e o daemon (spec ux-daemon.md) ----

    use crate::ipc::protocol::{CameraInfo, Request, Response};
    use crate::ui::daemon::{DaemonState, Modal, Mode, PendingRequest};

    /// Uma janela de verdade (sem iniciar pipelines) com uma câmera "Portão".
    fn test_app() -> App {
        let cam: crate::config::CameraConfig =
            toml::from_str("url = \"rtsp://127.0.0.1:9/x\"\nname = \"Portão\"").unwrap();
        let dir = std::env::temp_dir().join(format!("rrv-ui-test-{}", std::process::id()));
        // O português é o idioma dos testes (`new_app` aplica o do `view.toml` da pessoa).
        let (app, _task) = crate::ui::app::new_app(
            vec![cam],
            Default::default(),
            Default::default(),
            Default::default(),
            "cosmic".into(),
            crate::config::LogsConfigFile {
                dir: Some(dir),
                ..Default::default()
            },
            Vec::new(),
            Default::default(),
            Default::default(),
            Default::default(),
            crate::ui::DaemonOptions {
                embedded: true,
                socket: Some("/nonexistent/rrv.sock".into()),
                token: None,
            },
        );
        crate::i18n::set(crate::i18n::Lang::Pt);
        app
    }

    fn info(name: &str, recording: bool) -> CameraInfo {
        CameraInfo {
            index: 0,
            name: name.into(),
            status: "live".into(),
            enabled: true,
            recording,
            motion: false,
            stream: "main".into(),
            decoder: None,
            decoder_hw: false,
            ..Default::default()
        }
    }

    /// Põe a janela em modo "conectado" a um daemon que (não) existe de verdade:
    /// há um `DaemonLink`, então os pedidos saem, e o estado é o que o teste manda.
    fn connected(app: &mut App, recording: bool) {
        app.link = Some(crate::ipc::link::DaemonLink::spawn(
            "/nonexistent/rrv.sock".into(),
        ));
        app.daemon = DaemonState::connecting("/nonexistent/rrv.sock".into());
        app.daemon.mode = Mode::Connected;
        app.daemon.server = "rrv-daemon test".into();
        app.daemon.cameras = vec![info("Portão", recording)];
        app.engine.set_display_only(true);
        app.sidebar.selected = Some(0);
    }

    #[test]
    fn a_window_without_a_daemon_starts_in_the_local_engine() {
        let app = test_app();
        assert_eq!(app.daemon.mode, Mode::Embedded);
        assert!(app.link.is_none());
        assert!(!app.engine.display_only);
    }

    #[test]
    fn with_a_daemon_the_window_only_shows() {
        let mut app = test_app();
        connected(&mut app, false);
        assert!(app.engine.display_only, "a janela não grava nem detecta");
        assert!(app.engine.toggle_recording(0).is_err());
    }

    #[test]
    fn recording_is_only_shown_after_the_daemon_confirms() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::ToggleRecording);
        // o pedido saiu e está pendente: nada de REC ainda
        assert_eq!(app.pending.len(), 1);
        assert!(matches!(
            app.pending.values().next(),
            Some(PendingRequest::ToggleRecording { camera }) if camera == "Portão"
        ));
        assert!(!app.is_recording && !app.sidebar.cameras[0].is_recording);
        // apertar de novo enquanto espera não manda outro pedido
        let _ = update(&mut app, Message::ToggleRecording);
        assert_eq!(app.pending.len(), 1, "um pedido por vez");
    }

    #[test]
    fn a_daemon_error_becomes_a_toast_and_the_state_does_not_change() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::ToggleRecording);
        let token = *app.pending.keys().next().unwrap();
        let toasts_before = app.toasts.len();
        handle_reply(
            &mut app,
            token,
            Ok(Response::Error {
                message: "a câmera não está rodando".into(),
            }),
        );
        assert!(app.pending.is_empty());
        assert_eq!(app.toasts.len(), toasts_before + 1);
        assert!(
            app.toasts
                .last()
                .unwrap()
                .message
                .contains("Falha na gravação")
        );
        assert!(!app.is_recording, "nunca REC sem confirmação");
    }

    #[test]
    fn without_a_connection_nothing_is_sent_and_the_user_is_told() {
        let mut app = test_app();
        connected(&mut app, false);
        app.daemon.mode = Mode::Lost;
        let _ = update(&mut app, Message::ToggleRecording);
        assert!(app.pending.is_empty());
        assert!(!app.toasts.is_empty());
    }

    #[test]
    fn rec_comes_from_the_daemon_by_camera_name() {
        let mut app = test_app();
        connected(&mut app, true);
        app.engine.status[0] = sidebar::CameraStatus::Live;
        sync_status_rows(&mut app);
        assert!(app.sidebar.cameras[0].is_recording);
        assert_eq!(
            app.sidebar.cameras[0].status,
            sidebar::CameraStatus::Recording
        );
        // o daemon para de gravar: volta a ao vivo
        app.daemon.cameras[0].recording = false;
        sync_status_rows(&mut app);
        assert!(!app.sidebar.cameras[0].is_recording);
        assert_eq!(app.sidebar.cameras[0].status, sidebar::CameraStatus::Live);
        // uma câmera que o daemon não conhece não ganha REC
        app.daemon.cameras[0].name = "Outra".into();
        app.daemon.cameras[0].recording = true;
        sync_status_rows(&mut app);
        assert!(!app.sidebar.cameras[0].is_recording);
    }

    #[test]
    fn zones_stay_in_the_editor_when_the_daemon_refuses() {
        let mut app = test_app();
        connected(&mut app, false);
        let drawing = vec![
            crate::domain::zones::Point::new(0.1, 0.1),
            crate::domain::zones::Point::new(0.9, 0.1),
            crate::domain::zones::Point::new(0.5, 0.9),
        ];
        app.zone_edit = Some(crate::ui::app::ZoneEdit {
            camera_idx: 0,
            temp_vertices: drawing.clone(),
            saving: false,
        });
        finish_zone(&mut app);
        assert!(
            app.zone_edit.as_ref().unwrap().saving,
            "aguardando o daemon"
        );
        assert!(
            app.engine.zones[0].zones.is_empty(),
            "nada é aplicado antes da confirmação"
        );
        let token = *app.pending.keys().next().unwrap();
        handle_reply(
            &mut app,
            token,
            Ok(Response::Error {
                message: "zona inválida".into(),
            }),
        );
        let edit = app.zone_edit.as_ref().expect("o editor continua aberto");
        assert_eq!(edit.temp_vertices.len(), 3, "o desenho está intacto");
        assert!(!edit.saving);
        assert!(app.engine.zones[0].zones.is_empty());
    }

    #[test]
    fn zones_are_applied_and_the_drawing_cleared_when_the_daemon_confirms() {
        let mut app = test_app();
        connected(&mut app, false);
        app.zone_edit = Some(crate::ui::app::ZoneEdit {
            camera_idx: 0,
            temp_vertices: vec![
                crate::domain::zones::Point::new(0.1, 0.1),
                crate::domain::zones::Point::new(0.9, 0.1),
                crate::domain::zones::Point::new(0.5, 0.9),
            ],
            saving: false,
        });
        finish_zone(&mut app);
        let token = *app.pending.keys().next().unwrap();
        handle_reply(&mut app, token, Ok(Response::Ok));
        assert_eq!(app.engine.zones[0].zones.len(), 1);
        assert!(app.zone_edit.as_ref().unwrap().temp_vertices.is_empty());
    }

    fn seg(id: i64, start: i64, end: Option<i64>, file: &str) -> crate::ipc::protocol::SegmentInfo {
        crate::ipc::protocol::SegmentInfo {
            id,
            camera: "Portão".into(),
            ts_start: start,
            ts_end: end,
            bytes: 1,
            has_motion: false,
            protected: false,
            mode: "manual".into(),
            file: file.into(),
        }
    }

    fn history_reply(segs: Vec<crate::ipc::protocol::SegmentInfo>) -> Response {
        Response::History {
            segments: segs,
            events: Vec::new(),
            truncated: false,
        }
    }

    #[test]
    fn the_recordings_view_needs_the_daemon() {
        let mut app = test_app();
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        assert!(app.recordings.is_none(), "sem daemon não há histórico");
        assert!(
            app.toasts
                .iter()
                .any(|t| t.message.contains("motor local") && t.message.contains("RRV_SOCKET"))
        );
    }

    #[test]
    fn opening_the_recordings_view_asks_the_daemon_for_the_history() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let v = app.recordings.as_ref().expect("a vista abriu");
        assert!(v.loading);
        assert_eq!(v.lanes, ["Portão"], "uma faixa por câmera do daemon");
        assert!(
            app.pending
                .values()
                .any(|p| matches!(p, PendingRequest::History)),
            "o pedido de histórico saiu"
        );
        // a resposta preenche a vista
        let token = *app.pending.keys().next().unwrap();
        handle_reply(
            &mut app,
            token,
            Ok(history_reply(vec![seg(1, 1_000, Some(2_000), "a.mkv")])),
        );
        let v = app.recordings.as_ref().unwrap();
        assert!(!v.loading && v.error.is_none());
        assert_eq!(v.segments.len(), 1);
    }

    #[test]
    fn a_history_error_is_shown_not_swallowed() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        handle_reply(
            &mut app,
            token,
            Ok(Response::Error {
                message: "o daemon está sem histórico".into(),
            }),
        );
        let v = app.recordings.as_ref().unwrap();
        assert!(!v.loading);
        assert!(v.error.as_deref().unwrap().contains("sem histórico"));
    }

    #[test]
    fn clicking_a_missing_file_says_where_it_looked_and_starts_no_player() {
        let mut app = test_app();
        connected(&mut app, false);
        app.recordings_dir = std::env::temp_dir().join("rrv-nao-existe-mesmo");
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        handle_reply(
            &mut app,
            token,
            Ok(history_reply(vec![seg(
                1,
                now - 60_000,
                Some(now - 30_000),
                "a.mkv",
            )])),
        );
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked {
                lane: 0,
                t_ms: now - 45_000,
            }),
        );
        assert!(app.recordings.as_ref().unwrap().player.is_none());
        assert!(
            app.toasts
                .iter()
                .any(|t| t.message.contains("Arquivo não encontrado")
                    && t.message.contains("[recording] dir")),
            "{:?}",
            app.toasts.iter().map(|t| &t.message).collect::<Vec<_>>()
        );
    }

    #[test]
    fn clicking_where_nothing_was_recorded_says_so() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        handle_reply(&mut app, token, Ok(history_reply(vec![])));
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked { lane: 0, t_ms: 5 }),
        );
        assert!(
            app.toasts
                .iter()
                .any(|t| t.message.contains("Sem gravação"))
        );
    }

    #[test]
    fn exporting_without_marks_explains_what_to_do_and_sends_nothing() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        app.pending.clear();
        let _ = update(&mut app, Message::Recordings(RecMsg::Export));
        assert!(
            app.toasts
                .iter()
                .any(|t| t.message.contains("Marque o início"))
        );
        assert!(app.pending.is_empty());
    }

    #[test]
    fn the_recordings_view_owns_the_keyboard_and_escape_closes_it() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        // `r` normalmente grava; com a vista aberta não faz nada
        let _ = handle_key(
            &mut app,
            iced::keyboard::Key::Character("r".into()),
            iced::keyboard::Modifiers::default(),
        );
        assert!(app.recordings.is_some());
        assert!(
            !app.pending
                .values()
                .any(|p| matches!(p, PendingRequest::ToggleRecording { .. }))
        );
        let _ = handle_key(
            &mut app,
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            iced::keyboard::Modifiers::default(),
        );
        assert!(app.recordings.is_none());
    }

    #[test]
    fn the_padlock_only_shows_after_the_daemon_confirms() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        handle_reply(
            &mut app,
            token,
            Ok(history_reply(vec![seg(7, 1_000, Some(2_000), "a.mkv")])),
        );
        // nada tocando: avisa e não pede nada
        app.pending.clear();
        let _ = update(&mut app, Message::Recordings(RecMsg::ToggleProtect));
        assert!(app.pending.is_empty());
        // a resposta de uma proteção pedida muda o segmento só se for Ok
        handle_reply_for(
            &mut app,
            PendingRequest::Protect {
                segment_id: 7,
                protected: true,
            },
            Ok(Response::Error {
                message: "no".into(),
            }),
        );
        assert!(!app.recordings.as_ref().unwrap().segments[0].protected);
        handle_reply_for(
            &mut app,
            PendingRequest::Protect {
                segment_id: 7,
                protected: true,
            },
            Ok(Response::Ok),
        );
        assert!(app.recordings.as_ref().unwrap().segments[0].protected);
    }

    fn handle_reply_for(app: &mut App, pending: PendingRequest, result: Result<Response, String>) {
        let token = app.next_token;
        app.next_token += 1;
        app.pending.insert(token, pending);
        handle_reply(app, token, result);
    }

    #[test]
    fn a_disk_low_event_shows_a_toast_and_never_lands_on_a_camera() {
        let mut app = test_app();
        connected(&mut app, false);
        let before = app.sidebar.timeline.events_in_range(0, u64::MAX).len();
        daemon_event(
            &mut app,
            crate::ipc::protocol::WireEvent {
                camera: 0,
                name: "Disco".into(),
                kind: EventType::DiskLow,
                detail: Some("8% livre (40.0 GiB)".into()),
                notification: Some(("Disco quase cheio".into(), "x".into())),
                unix_secs: 1,
            },
        );
        assert!(
            app.toasts
                .iter()
                .any(|t| t.message.contains("quase cheio") && t.message.contains("8% livre"))
        );
        assert_eq!(
            app.sidebar.timeline.events_in_range(0, u64::MAX).len(),
            before
        );
        // a recuperação (sem notificação) não incomoda
        let n = app.toasts.len();
        daemon_event(
            &mut app,
            crate::ipc::protocol::WireEvent {
                camera: 0,
                name: "Disco".into(),
                kind: EventType::DiskLow,
                detail: Some("20% livre".into()),
                notification: None,
                unix_secs: 2,
            },
        );
        assert_eq!(app.toasts.len(), n);
    }

    #[test]
    fn clicking_an_event_opens_the_recording_5_seconds_before_it() {
        let mut app = test_app();
        connected(&mut app, false);
        app.recordings_dir = std::env::temp_dir().join("rrv-nao-existe-mesmo");
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        // o segmento cobre [now-60s, now-10s]; o evento é em now-40s
        handle_reply(
            &mut app,
            token,
            Ok(history_reply(vec![seg(
                1,
                now - 60_000,
                Some(now - 10_000),
                "a.mkv",
            )])),
        );
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::EventClicked {
                camera: "Portão".into(),
                ts_ms: now - 40_000,
            }),
        );
        // caiu dentro do segmento: tentou abrir o arquivo (que não existe nesta pasta)
        assert!(
            app.toasts
                .iter()
                .any(|t| t.message.contains("Arquivo não encontrado")),
            "{:?}",
            app.toasts.iter().map(|t| &t.message).collect::<Vec<_>>()
        );
        // câmera desconhecida: nada acontece
        let n = app.toasts.len();
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::EventClicked {
                camera: "Outra".into(),
                ts_ms: now,
            }),
        );
        assert_eq!(app.toasts.len(), n);
        let _ = update(&mut app, Message::Recordings(RecMsg::ToggleMotionOnly));
        assert!(app.recordings.as_ref().unwrap().motion_only);
    }

    fn history_requests(app: &App) -> usize {
        app.pending
            .values()
            .filter(|p| matches!(p, PendingRequest::History))
            .count()
    }

    #[test]
    fn the_open_view_refreshes_itself_and_follows_the_live_edge() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        handle_reply(
            &mut app,
            token,
            Ok(history_reply(vec![seg(1, 1_000, Some(2_000), "a.mkv")])),
        );
        assert_eq!(history_requests(&app), 0);

        // ainda não venceu: nada é pedido
        super::super::recordings::tick(&mut app);
        assert_eq!(history_requests(&app), 0, "5 s ainda não passaram");

        // venceu: pede de novo, e a janela anda com o relógio
        let old_to = app.recordings.as_ref().unwrap().span.to;
        app.recordings.as_mut().unwrap().refreshed_at = Instant::now() - Duration::from_secs(10);
        std::thread::sleep(Duration::from_millis(20));
        super::super::recordings::tick(&mut app);
        assert_eq!(history_requests(&app), 1);
        let v = app.recordings.as_ref().unwrap();
        assert!(v.span.to > old_to, "a janela seguiu o vivo");
        assert!(
            !v.loading,
            "o refresco não pisca \"Carregando…\" com dados na tela"
        );
    }

    #[test]
    fn looking_at_the_past_is_not_dragged_by_the_refresh() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        handle_reply(&mut app, token, Ok(history_reply(vec![])));
        let _ = update(&mut app, Message::Recordings(RecMsg::Pan(-0.5)));
        assert!(!app.recordings.as_ref().unwrap().follow);
        let span = app.recordings.as_ref().unwrap().span;
        app.pending.clear();
        app.recordings.as_mut().unwrap().refreshed_at = Instant::now() - Duration::from_secs(10);
        super::super::recordings::tick(&mut app);
        assert_eq!(history_requests(&app), 1, "ainda atualiza os dados");
        assert_eq!(
            app.recordings.as_ref().unwrap().span,
            span,
            "mas não move a janela"
        );
        // voltar ao presente religa o seguir
        let _ = update(&mut app, Message::Recordings(RecMsg::SetSpan(6)));
        assert!(app.recordings.as_ref().unwrap().follow);
    }

    #[test]
    fn without_a_connection_the_refresh_is_silent() {
        let mut app = test_app();
        connected(&mut app, false);
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        app.daemon.mode = Mode::Lost;
        app.pending.clear();
        let toasts = app.toasts.len();
        app.recordings.as_mut().unwrap().refreshed_at = Instant::now() - Duration::from_secs(10);
        super::super::recordings::tick(&mut app);
        assert!(app.pending.is_empty());
        assert_eq!(app.toasts.len(), toasts, "sem avisos a cada 5 s");
    }

    /// A 3 s mkv made on the spot, to give the player something real to open.
    fn tiny_mkv(path: &std::path::Path) {
        use gstreamer as gst;
        use gstreamer::prelude::*;
        let _ = gst::init();
        let p = gst::parse::launch(&format!(
            "videotestsrc num-buffers=90 ! video/x-raw,format=I420,width=160,height=120,framerate=30/1 \
             ! x264enc tune=zerolatency key-int-max=10 ! h264parse ! matroskamux ! filesink location={}",
            path.display()
        ))
        .unwrap();
        p.set_state(gst::State::Playing).unwrap();
        p.bus()
            .unwrap()
            .timed_pop_filtered(
                gst::ClockTime::from_seconds(20),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
            .unwrap();
        p.set_state(gst::State::Null).unwrap();
    }

    #[test]
    fn clicking_inside_the_playing_segment_seeks_instead_of_reopening_it() {
        let _media = crate::ui::test_support::media_gpu_lock();
        let mut app = test_app();
        connected(&mut app, false);
        let dir = std::env::temp_dir().join(format!("rrv-ui-drag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        tiny_mkv(&dir.join("a.mkv"));
        app.recordings_dir = dir.clone();
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let start = now - 120_000;
        handle_reply(
            &mut app,
            token,
            Ok(history_reply(vec![seg(
                1,
                start,
                Some(start + 3_000),
                "a.mkv",
            )])),
        );

        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked {
                lane: 0,
                t_ms: start + 100,
            }),
        );
        let first = app
            .recordings
            .as_ref()
            .unwrap()
            .player
            .as_ref()
            .expect("abriu")
            .bridge
            .clone();

        // arrastar: vários cliques dentro do mesmo trecho nunca reabrem o arquivo
        for dt in [1_000, 1_500, 2_000, 2_500] {
            let _ = update(
                &mut app,
                Message::Recordings(RecMsg::Clicked {
                    lane: 0,
                    t_ms: start + dt,
                }),
            );
            let p = app.recordings.as_ref().unwrap().player.as_ref().unwrap();
            assert!(
                std::sync::Arc::ptr_eq(&first, &p.bridge),
                "reabriu o arquivo em +{dt} ms"
            );
        }
        // e o seek vale: a posição chega perto do último ponto (ou fica pendente até a duração)
        let mut position = 0;
        for _ in 0..60 {
            super::super::recordings::tick(&mut app);
            position = app
                .recordings
                .as_ref()
                .unwrap()
                .player
                .as_ref()
                .unwrap()
                .bridge
                .lock()
                .unwrap()
                .playback_position_ms()
                .unwrap_or(0);
            if position >= 2_000 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            position >= 2_000,
            "posição depois de arrastar: {position} ms"
        );
        // nenhum aviso de "lacuna" por causa do arrastar
        assert!(
            !app.toasts
                .iter()
                .any(|t| t.message.contains("Sem gravação"))
        );
        let _ = update(&mut app, Message::Recordings(RecMsg::Close));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn seg_for(
        id: i64,
        camera: &str,
        start: i64,
        end: i64,
        file: &str,
    ) -> crate::ipc::protocol::SegmentInfo {
        let mut s = seg(id, start, Some(end), file);
        s.camera = camera.into();
        s
    }

    /// Duas câmeras (Portão e Garagem) com um arquivo de 3 s cada, na pasta de gravações.
    fn two_camera_app(
        tag: &str,
    ) -> (
        App,
        std::path::PathBuf,
        i64,
        std::sync::MutexGuard<'static, ()>,
    ) {
        let media = crate::ui::test_support::media_gpu_lock();
        let mut app = test_app();
        connected(&mut app, false);
        app.daemon.cameras.push(info("Garagem", false));
        let dir = std::env::temp_dir().join(format!("rrv-ui-multi-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        tiny_mkv(&dir.join("portao.mkv"));
        tiny_mkv(&dir.join("garagem.mkv"));
        app.recordings_dir = dir.clone();
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let start = now - 120_000;
        // Portão: [start, start+3 s]; Garagem: começa 1 s depois e dura 3 s (cobre [start+1, start+4])
        handle_reply(
            &mut app,
            token,
            Ok(history_reply(vec![
                seg_for(1, "Portão", start, start + 3_000, "portao.mkv"),
                seg_for(2, "Garagem", start + 1_000, start + 4_000, "garagem.mkv"),
            ])),
        );
        (app, dir, start, media)
    }

    #[test]
    fn a_follower_channel_opens_the_matching_segment_and_reports_a_gap() {
        let (mut app, dir, start, _media) = two_camera_app("follow");
        // principal: Portão em +2 s; liga a comparação com a Garagem
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked {
                lane: 0,
                t_ms: start + 2_000,
            }),
        );
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::ToggleCompare("Garagem".into())),
        );
        super::super::recordings::tick(&mut app);
        let v = app.recordings.as_ref().unwrap();
        assert_eq!(v.followers.len(), 1);
        let f = v.followers[0]
            .player
            .as_ref()
            .expect("o seguidor abriu o arquivo da Garagem");
        assert_eq!(f.segment.id, 2);
        assert!(!std::sync::Arc::ptr_eq(
            &f.bridge,
            &v.player.as_ref().unwrap().bridge
        ));

        // o principal vai para um instante em que a Garagem não gravava (antes de +1 s)
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked {
                lane: 0,
                t_ms: start + 100,
            }),
        );
        super::super::recordings::tick(&mut app);
        let f = &app.recordings.as_ref().unwrap().followers[0];
        assert!(f.player.is_none(), "sem gravação da Garagem em +0,1 s");
        assert_eq!(f.note.as_deref(), Some("Sem gravação neste instante"));

        // e quando volta a haver gravação, o seguidor reabre sozinho
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked {
                lane: 0,
                t_ms: start + 2_500,
            }),
        );
        // Abrir um arquivo é do GStreamer; com a máquina carregada pode levar mais de um tick (e a
        // tentativa seguinte espera um pouco): espera a reabertura em vez de exigi-la no primeiro tick.
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end
            && app.recordings.as_ref().unwrap().followers[0]
                .player
                .is_none()
        {
            super::super::recordings::tick(&mut app);
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(
            app.recordings.as_ref().unwrap().followers[0]
                .player
                .is_some(),
            "o seguidor não reabriu em 10 s"
        );
        let _ = update(&mut app, Message::Recordings(RecMsg::Close));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn controls_reach_every_channel_and_a_new_follower_inherits_them() {
        let (mut app, dir, start, _media) = two_camera_app("controls");
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked {
                lane: 0,
                t_ms: start + 2_000,
            }),
        );
        let _ = update(&mut app, Message::Recordings(RecMsg::Rate(2.0)));
        let _ = update(&mut app, Message::Recordings(RecMsg::PlayPause)); // pausa
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::ToggleCompare("Garagem".into())),
        );
        super::super::recordings::tick(&mut app);
        let v = app.recordings.as_ref().unwrap();
        let f = v.followers[0].player.as_ref().unwrap();
        assert!(f.paused, "o canal novo entra pausado como o principal");
        assert_eq!(f.rate, 2.0, "e na mesma velocidade");
        // retomar vale para os dois
        let _ = update(&mut app, Message::Recordings(RecMsg::PlayPause));
        let v = app.recordings.as_mut().unwrap();
        assert!(v.players_mut().all(|p| !p.paused));
        let _ = update(&mut app, Message::Recordings(RecMsg::Close));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clicking_a_followers_lane_makes_it_the_main_and_keeps_the_comparison() {
        let (mut app, dir, start, _media) = two_camera_app("swap");
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked {
                lane: 0,
                t_ms: start + 2_000,
            }),
        );
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::ToggleCompare("Garagem".into())),
        );
        super::super::recordings::tick(&mut app);
        // clica na faixa da Garagem (lane 1): ela vira a principal; o Portão passa a seguidor
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked {
                lane: 1,
                t_ms: start + 2_000,
            }),
        );
        let v = app.recordings.as_ref().unwrap();
        assert_eq!(v.player.as_ref().unwrap().camera, "Garagem");
        assert_eq!(v.followers.len(), 1);
        assert_eq!(v.followers[0].camera, "Portão");
        super::super::recordings::tick(&mut app);
        assert!(
            app.recordings.as_ref().unwrap().followers[0]
                .player
                .is_some()
        );
        // tirar da comparação; o principal não pode ser tirado
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::ToggleCompare("Garagem".into())),
        );
        assert_eq!(
            app.recordings.as_ref().unwrap().followers.len(),
            1,
            "o principal fica"
        );
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::ToggleCompare("Portão".into())),
        );
        assert!(app.recordings.as_ref().unwrap().followers.is_empty());
        let _ = update(&mut app, Message::Recordings(RecMsg::Close));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_follower_with_a_missing_file_says_so_and_does_not_hammer_the_disk() {
        let (mut app, dir, start, _media) = two_camera_app("missing");
        std::fs::remove_file(dir.join("garagem.mkv")).unwrap();
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::Clicked {
                lane: 0,
                t_ms: start + 2_000,
            }),
        );
        let _ = update(
            &mut app,
            Message::Recordings(RecMsg::ToggleCompare("Garagem".into())),
        );
        super::super::recordings::tick(&mut app);
        let f = &app.recordings.as_ref().unwrap().followers[0];
        assert!(f.player.is_none());
        assert!(
            f.note
                .as_deref()
                .unwrap()
                .contains("Arquivo não encontrado")
        );
        assert!(f.retry_at.is_some(), "a próxima tentativa fica para depois");
        let _ = update(&mut app, Message::Recordings(RecMsg::Close));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_window_finds_the_daemons_recordings_when_its_own_folder_is_wrong() {
        let mut app = test_app();
        connected(&mut app, false);
        let base = std::env::temp_dir().join(format!("rrv-ui-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (wrong, host) = (base.join("wrong"), base.join("host"));
        std::fs::create_dir_all(&wrong).unwrap();
        std::fs::create_dir_all(&host).unwrap();
        std::fs::write(host.join("a.mkv"), b"x").unwrap();
        // a pasta do compose, achada pela variável que o compose usa
        // SAFETY: only this test touches RRV_RECORDINGS in this process.
        unsafe { std::env::set_var("RRV_RECORDINGS", &host) };
        app.recordings_dir = wrong.clone();
        let _ = update(&mut app, Message::Recordings(RecMsg::Open));
        let token = *app.pending.keys().next().unwrap();
        handle_reply(
            &mut app,
            token,
            Ok(history_reply(vec![seg(1, 1_000, Some(2_000), "a.mkv")])),
        );
        unsafe { std::env::remove_var("RRV_RECORDINGS") };
        assert_eq!(
            app.recordings_dir, host,
            "a janela passou a ler a pasta certa"
        );
        assert!(
            app.toasts
                .iter()
                .any(|t| t.message.contains("encontradas em"))
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn closing_without_local_recordings_does_not_ask() {
        let mut app = test_app();
        let _ = update(&mut app, Message::QuitRequested);
        assert!(app.modal.is_none(), "nada gravando: fecha direto");
    }

    #[test]
    fn with_a_daemon_closing_never_asks() {
        let mut app = test_app();
        connected(&mut app, true);
        assert_eq!(local_recording_count(&app), 0);
        let _ = update(&mut app, Message::QuitRequested);
        assert!(app.modal.is_none(), "o daemon segue gravando");
    }

    #[test]
    fn a_modal_traps_the_keyboard_enter_accepts_esc_declines() {
        use iced::keyboard::{Key, Modifiers, key::Named};
        let mut app = test_app();
        app.modal = Some(Modal::Quit { recordings: 2 });
        // outras teclas não disparam nada na interface de trás
        let _ = handle_key(&mut app, Key::Character("r".into()), Modifiers::empty());
        assert!(app.modal.is_some() && app.pending.is_empty());
        // Esc recusa
        let _ = handle_key(&mut app, Key::Named(Named::Escape), Modifiers::empty());
        assert!(app.modal.is_none());
    }

    #[test]
    fn using_the_local_engine_takes_over_recording() {
        let mut app = test_app();
        connected(&mut app, true);
        app.pending.insert(
            9,
            PendingRequest::SetEnabled {
                camera: "Portão".into(),
            },
        );
        use_local_engine(&mut app);
        assert_eq!(app.daemon.mode, Mode::Embedded);
        assert!(app.link.is_none() && app.pending.is_empty());
        assert!(!app.engine.display_only, "agora a janela grava e detecta");
    }

    #[test]
    fn losing_the_daemon_never_switches_to_the_local_engine_by_itself() {
        let mut app = test_app();
        connected(&mut app, true);
        let _ = app.daemon.apply(crate::ipc::link::LinkEvent::Lost {
            reason: "x".into(),
            retry_in: std::time::Duration::from_secs(1),
        });
        assert_eq!(app.daemon.mode, Mode::Lost);
        assert!(app.engine.display_only, "segue só mostrando");
        assert!(app.daemon.banner().is_some());
    }

    #[test]
    fn requests_use_the_daemons_index_found_by_name() {
        let mut app = test_app();
        connected(&mut app, false);
        app.daemon.cameras[0].index = 7; // o daemon numera diferente
        assert_eq!(daemon_index(&mut app, "Portão"), Some(7));
        assert_eq!(daemon_index(&mut app, "Quintal"), None);
        let _ = Request::Status;
    }

    #[test]
    fn the_add_camera_wizard_opens_walks_through_its_stages_and_closes_with_esc() {
        use crate::ui::add_camera::{AddMsg, Stage};
        let mut app = test_app();
        assert!(app.add_camera.is_none());
        let _ = update(&mut app, Message::OpenAddCamera);
        assert!(matches!(
            app.add_camera.as_ref().map(|w| &w.stage),
            Some(Stage::Scanning)
        ));
        // a varredura devolve uma câmera; escolher abre o formulário
        let found = crate::onvif::Found {
            xaddr: "http://10.0.0.5/onvif/device_service".into(),
            ip: "10.0.0.5".into(),
            name: Some("IntelBras".into()),
            hardware: None,
        };
        let _ = update(&mut app, Message::AddCamera(AddMsg::Scanned(vec![found])));
        let _ = update(&mut app, Message::AddCamera(AddMsg::Pick(0)));
        assert!(matches!(
            app.add_camera.as_ref().map(|w| &w.stage),
            Some(Stage::Credentials { .. })
        ));
        // com o assistente aberto os atalhos não agem (digitar `f` na senha não abre o spotlight)
        let _ = handle_key(
            &mut app,
            iced::keyboard::Key::Character("f".into()),
            iced::keyboard::Modifiers::default(),
        );
        assert!(app.focus.is_normal());
        assert!(app.add_camera.is_some());
        // Esc fecha
        let _ = handle_key(
            &mut app,
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
            iced::keyboard::Modifiers::default(),
        );
        assert!(app.add_camera.is_none());
    }

    #[test]
    fn every_stage_of_the_add_camera_wizard_builds_a_view() {
        use crate::ui::add_camera::{AddCamera, Inspected, Stage};
        let app = test_app();
        let found = crate::onvif::Found {
            xaddr: "http://10.0.0.5/onvif/device_service".into(),
            ip: "10.0.0.5".into(),
            name: Some("IntelBras".into()),
            hardware: Some("iMX-C-309V".into()),
        };
        let stages = [
            Stage::Scanning,
            Stage::Pick { found: Vec::new() },
            Stage::Pick {
                found: vec![found.clone()],
            },
            Stage::Credentials {
                list: vec![found.clone()],
                found: found.clone(),
                user: "admin".into(),
                password: "x".into(),
                error: Some("erro".into()),
            },
            Stage::Inspecting {
                found: found.clone(),
                user: "admin".into(),
            },
            Stage::Done(Inspected {
                snippet: "[[cameras]]\nname = \"x\"\n".into(),
                secret: "cam_x_password".into(),
                stored: false,
            }),
        ];
        for stage in stages {
            let w = AddCamera { stage };
            let _ = crate::ui::view::add_camera::view(&app, &w);
        }
    }
}
