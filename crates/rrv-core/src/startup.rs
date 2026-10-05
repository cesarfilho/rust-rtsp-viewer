//! What every executable (the window and the headless daemon) does first:
//! read the config file, validate it and fold the global defaults into each
//! camera.

use crate::config::{CameraConfig, Config};
use crate::config_check::{check, has_errors};

/// Read and validate the config. Warnings go to stderr, prefixed with
/// `program`; any error (parse failure or an `erro:` issue) aborts with every
/// problem listed. Returns the config and the number of warnings.
pub fn load_config(config_path: &str, program: &str) -> Result<(Config, usize), String> {
    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| format!("cannot read {config_path}: {e}"))?;
    let (config, issues) =
        check(&config_str).map_err(|e| format!("cannot parse {config_path}: {e}"))?;
    for issue in &issues {
        eprintln!("{program}: {config_path}: {}", issue.render());
    }
    if has_errors(&issues) {
        return Err(format!("{config_path} has errors (see above)"));
    }
    Ok((config, issues.len()))
}

/// The top-level `latency_ms`, `decoder` and `do_retransmission` are defaults
/// each `[[cameras]]` entry may override.
pub fn merge_global_camera_defaults(cameras: &mut [CameraConfig], file_config: &Config) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Config {
        check(text).unwrap().0
    }

    #[test]
    fn global_defaults_fill_only_what_a_camera_left_unset() {
        let cfg = config(
            "latency_ms = 250\ndecoder = \"avdec_h264\"\n\
             [[cameras]]\nurl = \"rtsp://a/s\"\n\
             [[cameras]]\nurl = \"rtsp://b/s\"\nlatency_ms = 40\n",
        );
        let mut cams = cfg.cameras.clone().unwrap();
        merge_global_camera_defaults(&mut cams, &cfg);
        assert_eq!(cams[0].latency_ms, Some(250));
        assert_eq!(cams[0].decoder.as_deref(), Some("avdec_h264"));
        assert_eq!(cams[1].latency_ms, Some(40), "the camera's own value wins");
    }

    #[test]
    fn a_missing_file_is_a_clear_error() {
        let err = load_config("/nonexistent/rrv.toml", "test").unwrap_err();
        assert!(err.contains("cannot read /nonexistent/rrv.toml"), "{err}");
    }
}
