pub mod video_widget;
pub mod bridge;
pub mod pipeline;
pub mod theme;
pub mod sidebar;
pub mod grid;
pub mod zone_editor;
pub(crate) mod state;
pub(crate) mod update;
pub(crate) mod message;
pub(crate) mod app;
pub(crate) mod subscription;
pub(crate) mod view;
pub(crate) mod icons;

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
) -> iced::Result {
    let window = iced::window::Settings {
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

    iced::application("StreamView", update, view)
        .window(window)
        .font(icons::FONT_BYTES)
        .centered()
        .theme(|app: &App| app.theme.to_iced())
        .subscription(subscription)
        .run_with(move || {
            new_app(
                cameras,
                recording_config,
                snapshot_config,
                audio_config,
                theme_name,
                logs_config,
                groups,
                view_config,
                notify_config,
                motion_config,
            )
        })
}
