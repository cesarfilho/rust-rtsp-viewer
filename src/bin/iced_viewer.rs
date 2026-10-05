use std::process::ExitCode;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "rust-rtsp-viewer",
    version,
    about = "RTSP/HLS viewer with Iced GUI"
)]
struct Cli {
    /// Path to config file
    #[arg(default_value = "config.toml")]
    config: String,

    /// Validate the config file and exit (0 = usable, 1 = errors); opens no window
    #[arg(long)]
    check: bool,

    /// Use this window's own engine even if an rrv-daemon is running (it then
    /// records and detects only while the window is open)
    #[arg(long)]
    embedded: bool,

    /// Socket of the rrv-daemon (default: $RRV_SOCKET, else $XDG_RUNTIME_DIR/rrv/rrv.sock)
    #[arg(long, env = "RRV_SOCKET", value_name = "PATH")]
    daemon: Option<std::path::PathBuf>,
}

fn main() -> ExitCode {
    env_logger::init();
    let cli = Cli::parse();

    let result = if cli.check {
        check_only(&cli.config)
    } else {
        run(&cli.config, &cli)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rust-rtsp-viewer: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `--check`: validate and report, without starting GStreamer or the GUI.
fn check_only(config_path: &str) -> Result<(), String> {
    let (config, warnings) =
        rust_rtsp_viewer::startup::load_config(config_path, "rust-rtsp-viewer")?;
    let cameras = config.cameras.as_ref().map_or(0, Vec::len);
    println!("{config_path}: ok — {cameras} câmera(s), {warnings} aviso(s)");
    Ok(())
}

fn run(config_path: &str, cli: &Cli) -> Result<(), String> {
    let (config, _) = rust_rtsp_viewer::startup::load_config(config_path, "rust-rtsp-viewer")?;

    let mut cameras = config.cameras.clone().unwrap_or_default();
    rust_rtsp_viewer::startup::merge_global_camera_defaults(&mut cameras, &config);

    // Initialise GStreamer up front so a broken install fails with a clear
    // message here instead of panicking mid-construction once the GUI is up.
    gstreamer::init().map_err(|e| format!("GStreamer init failed: {e}"))?;

    let recording_config = config
        .recording
        .map(|r| r.into_config())
        .unwrap_or_default();
    let snapshot_config = config.snapshot.map(|s| s.into_config()).unwrap_or_default();
    let audio_config = config.audio.map(|a| a.into_config()).unwrap_or_default();
    let theme_name = config.theme.unwrap_or_default();
    let logs_config = config.logs.unwrap_or_default();
    let view_config = config.view.unwrap_or_default();
    let notify_config = config
        .notifications
        .map(|n| n.into_config())
        .unwrap_or_default();
    let motion_config = config.motion.map(|m| m.into_config()).unwrap_or_default();
    let groups: Vec<_> = config
        .groups
        .unwrap_or_default()
        .into_iter()
        .map(|g| g.into_group())
        .collect();

    rust_rtsp_viewer::ui::run(
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
        rust_rtsp_viewer::ui::DaemonOptions {
            embedded: cli.embedded,
            socket: cli.daemon.clone(),
        },
    )
    .map_err(|e| format!("GUI failed to start: {e}"))
}
