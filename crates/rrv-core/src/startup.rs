//! What every executable (the window and the headless daemon) does first:
//! read the config file, validate it and fold the global defaults into each
//! camera.

use crate::config::{CameraConfig, Config};
use crate::config_check::{check, has_errors};

/// Como tratar um `${SEGREDO}` que não existe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Secrets {
    /// Erro: o programa não deve subir com a câmera sem senha.
    Strict,
    /// Aviso, deixando o marcador no lugar: para `--check`, que valida o arquivo
    /// sem precisar dos segredos.
    Lenient,
}

/// Read and validate the config. Warnings go to stderr, prefixed with
/// `program`; any error (parse failure or an `erro:` issue, or a missing secret)
/// aborts with every problem listed. Returns the config and the number of warnings.
///
/// `${NOME}` in the URLs is replaced by the secret `NOME` (environment variable
/// or file in `/run/secrets`), see [`crate::secrets`].
pub fn load_config(config_path: &str, program: &str) -> Result<(Config, usize), String> {
    load_config_with(config_path, program, Secrets::Strict)
}

/// Como [`load_config`], escolhendo o tratamento de segredos ausentes.
pub fn load_config_with(
    config_path: &str,
    program: &str,
    secrets: Secrets,
) -> Result<(Config, usize), String> {
    let config_str = std::fs::read_to_string(config_path)
        .map_err(|e| format!("cannot read {config_path}: {e}"))?;
    let (mut config, issues) =
        check(&config_str).map_err(|e| format!("cannot parse {config_path}: {e}"))?;
    for issue in &issues {
        eprintln!("{program}: {config_path}: {}", issue.render());
    }
    if has_errors(&issues) {
        return Err(format!("{config_path} has errors (see above)"));
    }
    let mut warnings = issues.len();
    let missing = resolve_secrets(&mut config);
    if !missing.is_empty() {
        match secrets {
            Secrets::Strict => return Err(missing.join("\n")),
            Secrets::Lenient => {
                for m in &missing {
                    eprintln!("{program}: {config_path}: aviso: {m}");
                }
                warnings += missing.len();
            }
        }
    }
    Ok((config, warnings))
}

/// Expande os `${SEGREDO}` das URLs (câmeras, sub-stream, webhook). Devolve uma
/// mensagem por campo com segredo ausente; os marcadores ausentes ficam no lugar.
pub fn resolve_secrets(config: &mut Config) -> Vec<String> {
    use crate::secrets::{expand, lookup, missing_message};
    let mut problems = Vec::new();
    let mut fix = |field: String, text: &mut String| match expand(text, &lookup) {
        Ok(v) => *text = v,
        Err(names) => problems.push(missing_message(&field, &names)),
    };
    if let Some(cams) = config.cameras.as_mut() {
        for (i, cam) in cams.iter_mut().enumerate() {
            let who = cam
                .name
                .clone()
                .or_else(|| cam.label.clone())
                .map_or(format!("cameras[{i}]"), |n| format!("cameras[{i}] ({n})"));
            fix(format!("{who}.url"), &mut cam.url);
            if let Some(sub) = cam.sub_url.as_mut() {
                fix(format!("{who}.sub_url"), sub);
            }
        }
    }
    if let Some(url) = config.webhook.as_mut().and_then(|w| w.url.as_mut()) {
        fix("webhook.url".into(), url);
    }
    problems
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

    fn cfg(text: &str) -> Config {
        check(text).unwrap().0
    }

    #[test]
    fn secrets_are_expanded_in_cameras_sub_urls_and_the_webhook() {
        // SAFETY: nomes exclusivos deste teste.
        unsafe {
            std::env::set_var("RRV_T_CAM", "p@ss");
            std::env::set_var("RRV_T_HOOK", "tok123");
        }
        let mut c = cfg(
            "[[cameras]]\nurl = \"rtsp://admin:${RRV_T_CAM}@10.0.0.2/main\"\nname = \"Portão\"\n\
             sub_url = \"rtsp://admin:${RRV_T_CAM}@10.0.0.2/sub\"\n\
             [webhook]\nurl = \"https://ntfy.sh/${RRV_T_HOOK}\"\n",
        );
        assert!(resolve_secrets(&mut c).is_empty());
        let cam = &c.cameras.as_ref().unwrap()[0];
        assert_eq!(cam.url, "rtsp://admin:p%40ss@10.0.0.2/main");
        assert_eq!(
            cam.sub_url.as_deref(),
            Some("rtsp://admin:p%40ss@10.0.0.2/sub")
        );
        assert_eq!(
            c.webhook.unwrap().url.as_deref(),
            Some("https://ntfy.sh/tok123")
        );
    }

    #[test]
    fn a_missing_secret_names_the_field_and_the_secret_but_never_a_value() {
        let mut c =
            cfg("[[cameras]]\nurl = \"rtsp://admin:${RRV_T_AUSENTE}@h/s\"\nname = \"Garagem\"\n");
        let problems = resolve_secrets(&mut c);
        assert_eq!(problems.len(), 1);
        assert!(
            problems[0].contains("cameras[0] (Garagem).url"),
            "{}",
            problems[0]
        );
        assert!(problems[0].contains("'RRV_T_AUSENTE'"), "{}", problems[0]);
        // o marcador fica no lugar (modo leniente)
        assert!(c.cameras.unwrap()[0].url.contains("${RRV_T_AUSENTE}"));
    }

    #[test]
    fn strict_loading_fails_and_lenient_loading_warns() {
        let dir = std::env::temp_dir().join(format!("rrv-startup-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.toml");
        std::fs::write(
            &path,
            "[[cameras]]\nurl = \"rtsp://u:${RRV_T_SEM_SEGREDO}@h/s\"\nname = \"A\"\n",
        )
        .unwrap();
        let p = path.to_string_lossy().to_string();
        let err = load_config(&p, "t").unwrap_err();
        assert!(err.contains("RRV_T_SEM_SEGREDO"), "{err}");
        let (_, warnings) = load_config_with(&p, "t", Secrets::Lenient).unwrap();
        assert!(warnings >= 1, "o --check avisa em vez de falhar");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
