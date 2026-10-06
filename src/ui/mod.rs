// The engine moved to `crate::engine`; these keep the old `ui::` paths working.
pub use crate::engine::{bridge, pipeline};

pub mod add_camera;
pub(crate) mod app;
pub mod daemon;
pub mod detections_overlay;
pub mod grid;
pub(crate) mod icons;
pub(crate) mod message;
pub mod recordings;
pub mod sidebar;
pub(crate) mod state;
pub(crate) mod subscription;
#[cfg(test)]
pub mod test_support;
pub mod theme;
pub(crate) mod update;
pub mod video_shader;
pub mod video_widget;
pub(crate) mod view;
pub mod zone_editor;

pub(crate) use app::App;
pub(crate) use message::Message;

use app::new_app;
use subscription::subscription;
use update::update;
use view::view;

/// Wayland `app_id` / X11 `WM_CLASS`. Must match the basename of the installed
/// `.desktop` file so COSMIC (and GNOME) can attach the icon, group windows,
/// and show the real application name in the overview / dock.
pub const APP_ID: &str = "rust-rtsp-viewer";

/// Como a janela trata o daemon (flags `--embedded` e `--daemon`).
#[derive(Debug, Clone, Default)]
pub struct DaemonOptions {
    /// Força o motor local, mesmo que haja um daemon.
    pub embedded: bool,
    /// Socket do daemon, ou `tcp://host:porta` para um daemon em outra máquina; padrão: `$RRV_SOCKET`
    /// ou `$XDG_RUNTIME_DIR/rrv/rrv.sock`.
    pub socket: Option<std::path::PathBuf>,
    /// O token do daemon pela rede (ignorado num socket local).
    pub token: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    cameras: Vec<crate::config::CameraConfig>,
    recording_config: crate::domain::recording::RecordingConfig,
    snapshot_config: crate::domain::snapshot::SnapshotConfig,
    audio_config: crate::domain::audio::AudioConfig,
    theme_name: String,
    logs_config: crate::config::LogsConfigFile,
    groups: Vec<crate::domain::groups::CameraGroup>,
    view_config: crate::config::ViewConfigFile,
    notify_config: crate::domain::notify::NotifyConfig,
    motion_config: crate::domain::motion::MotionConfig,
    daemon: DaemonOptions,
) -> iced::Result {
    let window = iced::window::Settings {
        // The window's close button asks first when it would cut off a recording.
        exit_on_close_request: false,
        size: iced::Size::new(1280.0, 720.0),
        // COSMIC tiles and half-snaps aggressively; below this the toolbar and
        // sidebar stop being usable, so let the compositor clamp instead.
        min_size: Some(iced::Size::new(880.0, 560.0)),
        platform_specific: iced::window::settings::PlatformSpecific {
            application_id: APP_ID.to_string(),
            ..Default::default()
        },
        ..Default::default()
    };

    iced::application(
        move || {
            new_app(
                cameras.clone(),
                recording_config.clone(),
                snapshot_config.clone(),
                audio_config.clone(),
                theme_name.clone(),
                logs_config.clone(),
                groups.clone(),
                view_config.clone(),
                notify_config.clone(),
                motion_config.clone(),
                daemon.clone(),
            )
        },
        update,
        view,
    )
    .title("StreamView") // i18n-ok: nome próprio
    .window(window)
    .font(icons::FONT_BYTES)
    .centered()
    .theme(|app: &App| app.theme.to_iced())
    .subscription(subscription)
    .run()
}
