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
}

fn main() -> ExitCode {
    env_logger::init();
    let cli = Cli::parse();

    let result = if cli.check {
        check_only(&cli.config)
    } else {
        run(&cli.config)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("rust-rtsp-viewer: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Read and validate the config. Warnings are printed to stderr; any error
/// (parse failure or an `erro:` issue) aborts with every problem listed.
fn load_config(config_path: &str) -> Result<(rust_rtsp_viewer::config::Config, usize), String> {
    use rust_rtsp_viewer::config_check::{check, has_errors};

    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| format!("cannot read {config_path}: {e}"))?;
    let (config, issues) =
        check(&config_str).map_err(|e| format!("cannot parse {config_path}: {e}"))?;
    for issue in &issues {
        eprintln!("rust-rtsp-viewer: {config_path}: {}", issue.render());
    }
    if has_errors(&issues) {
        return Err(format!("{config_path} has errors (see above)"));
    }
    Ok((config, issues.len()))
}

/// `--check`: validate and report, without starting GStreamer or the GUI.
fn check_only(config_path: &str) -> Result<(), String> {
    let (config, warnings) = load_config(config_path)?;
    let cameras = config.cameras.as_ref().map_or(0, Vec::len);
    println!("{config_path}: ok — {cameras} câmera(s), {warnings} aviso(s)");
    Ok(())
}

fn run(config_path: &str) -> Result<(), String> {
    let (config, _) = load_config(config_path)?;

    let mut cameras = config.cameras.clone().unwrap_or_default();
    merge_global_camera_defaults(&mut cameras, &config);

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
    )
    .map_err(|e| format!("GUI failed to start: {e}"))
}

fn merge_global_camera_defaults(
    cameras: &mut [rust_rtsp_viewer::config::CameraConfig],
    file_config: &rust_rtsp_viewer::config::Config,
) {
    for cam in cameras.iter_mut() {
        if cam.decoder.is_none() {
            cam.decoder = file_config.decoder.clone();
        }
        if cam.do_retransmission.is_none() {
            cam.do_retransmission = file_config.do_retransmission;
        }
        if cam.latency_ms.is_none() {
            cam.latency_ms = file_config.latency_ms;
        }
    }
}
