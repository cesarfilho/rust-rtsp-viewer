//! Validation of `config.toml` beyond what `serde` checks.
//!
//! `toml` already reports a wrong *type* with line, column and field. What it
//! cannot see is a **typo** (`sub_ul`, silently ignored) or a value that parses
//! but makes no sense (latency 0, a group pointing at camera 9 of 3, a theme
//! that does not exist and quietly falls back to the default). [`check`]
//! reports those as [`Issue`]s with the path of the offending key, for the
//! normal startup (warnings on stderr) and for `--check`.
//!
//! Messages never contain credentials: URLs go through
//! [`mask_credentials`](crate::domain::redact::mask_credentials).

use std::collections::HashMap;

use crate::config::{CameraConfig, Config};
use crate::domain::redact::mask_credentials;

/// Themes `ui::app::new_app` understands (anything else falls back to the default).
pub const KNOWN_THEMES: [&str; 5] = ["cosmic", "dark", "light", "amoled", "opencode"];

/// URL schemes the pipelines handle: `rtsp(s)` via `rtspsrc`, `http(s)` via
/// `uridecodebin3`. Anything else is treated as a local file path.
const STREAM_SCHEMES: [&str; 4] = ["rtsp", "rtsps", "http", "https"];

/// Highest `latency_ms` that is plausible for a live view.
const MAX_REASONABLE_LATENCY_MS: u32 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The app cannot run with this config.
    Error,
    /// The app runs, but the setting is probably not what the user meant.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub severity: Severity,
    /// Where: `cameras[1] (Garagem).sub_url`, `motion.threshold`, …
    pub path: String,
    pub message: String,
}

impl Issue {
    fn error(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            path: path.into(),
            message: message.into(),
        }
    }

    fn warning(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            path: path.into(),
            message: message.into(),
        }
    }

    /// `erro: cameras[0].url: …` / `aviso: …`
    pub fn render(&self) -> String {
        let label = match self.severity {
            Severity::Error => "erro",
            Severity::Warning => "aviso",
        };
        format!("{label}: {}: {}", self.path, self.message)
    }
}

pub fn has_errors(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.severity == Severity::Error)
}

/// Parse `text` and validate it. `Err` is a parse failure (already carrying
/// line/column from `toml`); `Ok` carries the config plus every issue found.
pub fn check(text: &str) -> Result<(Config, Vec<Issue>), String> {
    let mut unknown = Vec::new();
    let de = toml::Deserializer::new(text);
    let config: Config = serde_ignored::deserialize(de, |path| unknown.push(path.to_string()))
        .map_err(|e| e.to_string())?;

    let mut issues: Vec<Issue> = unknown
        .into_iter()
        .map(|path| {
            Issue::warning(
                pretty_path(&path),
                "chave desconhecida (erro de digitação?); será ignorada",
            )
        })
        .collect();
    issues.extend(validate(&config));
    Ok((config, issues))
}

/// `serde_ignored` paths look like `cameras.?.0.sub_ul` (`?` = an `Option`,
/// a bare number = a sequence index). Render them as `cameras[0].sub_ul`, the
/// same shape as the other messages.
fn pretty_path(raw: &str) -> String {
    let mut out = String::new();
    for seg in raw.split('.').filter(|s| *s != "?") {
        if seg.chars().all(|c| c.is_ascii_digit()) && !seg.is_empty() {
            out.push_str(&format!("[{seg}]"));
        } else {
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(seg);
        }
    }
    out
}

/// Semantic checks on an already-parsed config.
pub fn validate(config: &Config) -> Vec<Issue> {
    let mut issues = Vec::new();

    if let Some(theme) = &config.theme
        && !KNOWN_THEMES.contains(&theme.trim().to_lowercase().as_str())
    {
        issues.push(Issue::warning(
            "theme",
            format!(
                "tema {theme:?} desconhecido; usando o padrão. Válidos: {}",
                KNOWN_THEMES.join(", ")
            ),
        ));
    }
    check_latency("latency_ms", config.latency_ms, &mut issues);
    if let Some(d) = &config.decoder
        && d.trim().is_empty()
    {
        issues.push(Issue::error(
            "decoder",
            "vazio; omita a chave para usar o padrão (decodebin)",
        ));
    }

    let cameras = config.cameras.as_deref().unwrap_or_default();
    if cameras.is_empty() {
        issues.push(Issue::error(
            "cameras",
            "nenhuma câmera configurada; adicione ao menos uma seção [[cameras]] com `url`",
        ));
    }
    for (i, cam) in cameras.iter().enumerate() {
        validate_camera(i, cam, &mut issues);
    }
    check_duplicates(cameras, &mut issues);

    for (g, group) in config
        .groups
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let path = format!("groups[{g}]");
        if group.name.trim().is_empty() {
            issues.push(Issue::warning(&path, "grupo sem nome"));
        }
        for &idx in &group.cameras {
            if idx >= cameras.len() {
                issues.push(Issue::warning(
                    &path,
                    format!(
                        "aponta para a câmera {idx}, mas só há {} (índices começam em 0)",
                        cameras.len()
                    ),
                ));
            }
        }
    }

    if let Some(m) = &config.motion {
        if m.threshold == Some(0) {
            issues.push(Issue::warning(
                "motion.threshold",
                "0 conta qualquer ruído como movimento (use 1–255)",
            ));
        }
        if let Some(a) = m.contour_area
            && !(a > 0.0 && a <= 1.0)
        {
            issues.push(Issue::warning(
                "motion.contour_area",
                format!("{a} fora de (0, 1]"),
            ));
        }
        if let Some(l) = m.lightning_threshold
            && !(0.0..=1.0).contains(&l)
        {
            issues.push(Issue::warning(
                "motion.lightning_threshold",
                format!("{l} fora de 0–1 (0 desliga)"),
            ));
        }
        if let Some(s) = m.sample_stride
            && !(1..=32).contains(&s)
        {
            issues.push(Issue::warning(
                "motion.sample_stride",
                format!("{s} fora de 1–32"),
            ));
        }
    }
    if let Some(d) = &config.daemon {
        let token = d.token.as_deref().unwrap_or("");
        if let Some(l) = &d.listen {
            match l.parse::<std::net::SocketAddr>() {
                Err(_) => issues.push(Issue::error(
                    "daemon.listen",
                    format!("'{l}' não é um endereço IP:porta (ex.: 0.0.0.0:7878)"),
                )),
                Ok(addr) => {
                    if let Err(m) = crate::ipc::auth::validate_token(token) {
                        issues.push(Issue::error(
                            "daemon.token",
                            format!("{m} (necessário com listen)"),
                        ));
                    } else if !addr.ip().is_loopback() {
                        issues.push(Issue::warning(
                            "daemon.listen",
                            "o canal pela rede não é criptografado: use só numa LAN de confiança ou por VPN"
                                .to_string(),
                        ));
                    }
                }
            }
        }
        if let Some(f) = &d.files_listen
            && f.parse::<std::net::SocketAddr>().is_err()
        {
            issues.push(Issue::error(
                "daemon.files_listen",
                format!("'{f}' não é um endereço IP:porta"),
            ));
        }
        if let Some(a) = &d.address
            && let Some(hostport) = a.strip_prefix(crate::ipc::client::TCP_PREFIX)
        {
            if !hostport.contains(':') {
                issues.push(Issue::error(
                    "daemon.address",
                    format!("'{a}' precisa de porta (tcp://host:porta)"),
                ));
            }
            if !token.is_empty()
                && let Err(m) = crate::ipc::auth::validate_token(token)
            {
                issues.push(Issue::error("daemon.token", m));
            }
        }
    }
    if let Some(m) = &config.mqtt
        && let Err(message) = m.clone().into_config()
    {
        issues.push(Issue::error("mqtt", message));
    }
    if let Some(d) = &config.detect {
        match d.clone().into_config() {
            Err(message) => issues.push(Issue::error("detect", message)),
            Ok(c) if c.enabled => {
                if !c.model.exists() {
                    issues.push(Issue::warning(
                        "detect.model",
                        format!(
                            "{} não existe (rode scripts/fetch-model.sh); a detecção ficará desligada",
                            c.model.display()
                        ),
                    ));
                }
                if !config
                    .motion
                    .as_ref()
                    .is_some_and(|m| m.enabled == Some(true))
                {
                    issues.push(Issue::warning(
                        "detect.enabled",
                        "a detecção só roda onde há movimento: ligue [motion] enabled".to_string(),
                    ));
                }
            }
            Ok(_) => {}
        }
    }
    if let Some(r) = &config.retention
        && let Err(message) = r.clone().into_config()
    {
        issues.push(Issue::error("retention", message));
    }
    if let Some(w) = &config.webhook
        && let Err(message) = w.clone().into_config()
    {
        issues.push(Issue::error("webhook", message));
    }
    if let Some(n) = &config.notifications
        && let Some(c) = n.cooldown_secs
        && c < crate::domain::notify::MIN_COOLDOWN_SECS
    {
        issues.push(Issue::warning(
            "notifications.cooldown_secs",
            format!(
                "{c} é menor que o mínimo ({}); será elevado",
                crate::domain::notify::MIN_COOLDOWN_SECS
            ),
        ));
    }
    if let Some(a) = &config.audio {
        check_volume("audio.volume", a.volume, &mut issues);
    }
    if let Some(s) = &config.snapshot
        && let Some(q) = s.quality
        && !(1..=100).contains(&q)
    {
        issues.push(Issue::warning(
            "snapshot.quality",
            format!("{q} fora de 1–100"),
        ));
    }
    if let Some(r) = &config.recording
        && let Some(c) = &r.container
        && !matches!(c.trim().to_lowercase().as_str(), "mkv" | "mp4")
    {
        issues.push(Issue::warning(
            "recording.container",
            format!("{c:?} desconhecido; usando mkv (válidos: mkv, mp4)"),
        ));
    }
    if let Some(v) = &config.view {
        if let Some(mode) = &v.mode
            && crate::domain::view::GridMode::parse(mode).is_none()
        {
            issues.push(Issue::warning(
                "view.mode",
                format!("{mode:?} inválido; use \"auto\", \"2x2\", \"3x3\"…"),
            ));
        }
        if let Some(l) = &v.layout
            && !matches!(l.trim().to_lowercase().as_str(), "grid" | "flex")
        {
            issues.push(Issue::warning(
                "view.layout",
                format!("{l:?} inválido; use \"grid\" ou \"flex\""),
            ));
        }
    }

    issues
}

fn camera_path(i: usize, cam: &CameraConfig) -> String {
    match cam.label.as_deref().or(cam.name.as_deref()) {
        Some(n) => format!("cameras[{i}] ({n})"),
        None => format!("cameras[{i}]"),
    }
}

fn validate_camera(i: usize, cam: &CameraConfig, issues: &mut Vec<Issue>) {
    let path = camera_path(i, cam);
    if cam.url.trim().is_empty() {
        issues.push(Issue::error(format!("{path}.url"), "vazia"));
    } else {
        check_stream_url(&format!("{path}.url"), &cam.url, issues);
    }
    if let Some(sub) = &cam.sub_url {
        let p = format!("{path}.sub_url");
        if sub.trim().is_empty() {
            issues.push(Issue::error(
                &p,
                "vazia; omita a chave se não há sub-stream",
            ));
        } else {
            check_stream_url(&p, sub, issues);
            if sub == &cam.url {
                issues.push(Issue::warning(&p, "igual à url principal; não reduz nada"));
            }
        }
    }
    check_latency(&format!("{path}.latency_ms"), cam.latency_ms, issues);
    check_volume(&format!("{path}.audio_volume"), cam.audio_volume, issues);
    if let Some(d) = &cam.decoder
        && d.trim().is_empty()
    {
        issues.push(Issue::error(
            format!("{path}.decoder"),
            "vazio; omita a chave para usar o padrão",
        ));
    }
}

fn check_stream_url(path: &str, url: &str, issues: &mut Vec<Issue>) {
    let scheme = url.split_once("://").map(|(s, _)| s.to_lowercase());
    match scheme {
        Some(s) if STREAM_SCHEMES.contains(&s.as_str()) => {}
        Some(s) => issues.push(Issue::warning(
            path,
            format!(
                "esquema {s:?} não suportado em {}; será tratado como arquivo local (suportados: {})",
                mask_credentials(url),
                STREAM_SCHEMES.join(", ")
            ),
        )),
        None => issues.push(Issue::warning(
            path,
            format!("{:?} não parece uma URL; será aberta como arquivo local", mask_credentials(url)),
        )),
    }
}

fn check_latency(path: &str, value: Option<u32>, issues: &mut Vec<Issue>) {
    if let Some(ms) = value
        && ms > MAX_REASONABLE_LATENCY_MS
    {
        issues.push(Issue::warning(
            path,
            format!("{ms} ms é muito alto para visualização ao vivo (máximo razoável: {MAX_REASONABLE_LATENCY_MS})"),
        ));
    }
}

fn check_volume(path: &str, value: Option<f32>, issues: &mut Vec<Issue>) {
    if let Some(v) = value
        && !(0.0..=1.0).contains(&v)
    {
        issues.push(Issue::warning(
            path,
            format!("{v} fora de 0.0–1.0; será limitado"),
        ));
    }
}

/// Two cameras with the same alias or label are ambiguous (the alias resolver
/// and the per-camera log file both key on them).
fn check_duplicates(cameras: &[CameraConfig], issues: &mut Vec<Issue>) {
    let mut seen: HashMap<(&'static str, String), usize> = HashMap::new();
    for (i, cam) in cameras.iter().enumerate() {
        for (field, value) in [("name", &cam.name), ("label", &cam.label)] {
            let Some(v) = value.as_deref().map(|v| v.trim().to_lowercase()) else {
                continue;
            };
            if v.is_empty() {
                continue;
            }
            if let Some(&first) = seen.get(&(field, v.clone())) {
                issues.push(Issue::warning(
                    format!("{}.{field}", camera_path(i, &cameras[i])),
                    format!("repete o de cameras[{first}]; os dois ficam ambíguos"),
                ));
            } else {
                seen.insert((field, v), i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(issues: &[Issue]) -> Vec<&str> {
        issues.iter().map(|i| i.path.as_str()).collect()
    }

    fn one_camera(extra: &str) -> String {
        format!("[[cameras]]\nurl = \"rtsp://h/s\"\n{extra}")
    }

    #[test]
    fn a_clean_config_has_no_issues() {
        let (_, issues) = check(&one_camera("")).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn the_shipped_example_has_no_issues() {
        let (_, issues) = check(include_str!("../../../config.toml.example")).unwrap();
        assert!(issues.is_empty(), "{issues:?}");
    }

    #[test]
    fn typos_are_reported_with_their_path() {
        let (_, issues) = check(&one_camera(
            "sub_ul = \"rtsp://h/sub\"\n[motion]\nthreshhold = 5\n",
        ))
        .unwrap();
        let p = paths(&issues);
        assert!(p.contains(&"cameras[0].sub_ul"), "{p:?}");
        assert!(p.contains(&"motion.threshhold"), "{p:?}");
        assert!(issues.iter().all(|i| i.severity == Severity::Warning));
    }

    #[test]
    fn a_wrong_type_is_a_parse_error_with_line_and_column() {
        let err = check(&one_camera("latency_ms = \"abc\"\n")).unwrap_err();
        assert!(
            err.contains("line 3") && err.contains("latency_ms"),
            "{err}"
        );
    }

    #[test]
    fn no_cameras_is_an_error() {
        let (_, issues) = check("theme = \"dark\"\n").unwrap();
        assert!(has_errors(&issues));
        assert_eq!(paths(&issues), ["cameras"]);
    }

    #[test]
    fn empty_urls_are_errors() {
        let (_, issues) = check("[[cameras]]\nurl = \"\"\nsub_url = \" \"\n").unwrap();
        assert!(has_errors(&issues));
        let p = paths(&issues);
        assert!(
            p.contains(&"cameras[0].url") && p.contains(&"cameras[0].sub_url"),
            "{p:?}"
        );
    }

    #[test]
    fn unsupported_schemes_warn_and_never_leak_the_password() {
        let (_, issues) = check("[[cameras]]\nurl = \"rtmp://user:segredo@h/s\"\n").unwrap();
        let msg = &issues[0].message;
        assert!(msg.contains("rtmp"), "{msg}");
        assert!(!msg.contains("segredo"), "senha vazou: {msg}");
    }

    #[test]
    fn a_sub_url_equal_to_the_main_url_warns() {
        let (_, issues) = check(&one_camera("sub_url = \"rtsp://h/s\"\n")).unwrap();
        assert_eq!(paths(&issues), ["cameras[0].sub_url"]);
    }

    #[test]
    fn out_of_range_values_warn() {
        let text = "theme = \"neon\"\nlatency_ms = 60000\n\
                    [[cameras]]\nurl = \"rtsp://h/s\"\naudio_volume = 2.5\n\
                    [motion]\nthreshold = 0\ncontour_area = 3.0\nsample_stride = 99\nlightning_threshold = 2.0\n\
                    [notifications]\ncooldown_secs = 1\n\
                    [view]\nmode = \"7x\"\nlayout = \"mosaic\"\n\
                    [recording]\ncontainer = \"avi\"\n\
                    [snapshot]\nquality = 0\n";
        let (_, issues) = check(text).unwrap();
        let p = paths(&issues);
        for want in [
            "theme",
            "latency_ms",
            "cameras[0].audio_volume",
            "motion.threshold",
            "motion.contour_area",
            "motion.sample_stride",
            "motion.lightning_threshold",
            "notifications.cooldown_secs",
            "view.mode",
            "view.layout",
            "recording.container",
            "snapshot.quality",
        ] {
            assert!(p.contains(&want), "faltou {want}: {p:?}");
        }
        assert!(!has_errors(&issues));
    }

    #[test]
    fn groups_pointing_past_the_last_camera_warn() {
        let (_, issues) =
            check(&one_camera("[[groups]]\nname = \"g\"\ncameras = [0, 3]\n")).unwrap();
        assert_eq!(paths(&issues), ["groups[0]"]);
        assert!(issues[0].message.contains('3'));
    }

    #[test]
    fn duplicate_names_and_labels_warn_once_each() {
        let text = "[[cameras]]\nurl = \"rtsp://a/s\"\nname = \"Frente\"\nlabel = \"Garagem\"\n\
                    [[cameras]]\nurl = \"rtsp://b/s\"\nname = \"frente\"\nlabel = \"Quintal\"\n";
        let (_, issues) = check(text).unwrap();
        assert_eq!(issues.len(), 1, "{issues:?}");
        assert!(issues[0].path.ends_with(".name"));
    }

    #[test]
    fn pretty_path_drops_option_markers_and_brackets_indices() {
        assert_eq!(pretty_path("cameras.?.0.sub_ul"), "cameras[0].sub_ul");
        assert_eq!(pretty_path("motion.?.threshhold"), "motion.threshhold");
        assert_eq!(pretty_path("groups.?.1.nome"), "groups[1].nome");
        assert_eq!(pretty_path("theme"), "theme");
    }

    #[test]
    fn render_marks_errors_and_warnings() {
        assert_eq!(Issue::error("a", "b").render(), "erro: a: b");
        assert_eq!(Issue::warning("a", "b").render(), "aviso: a: b");
    }
}
