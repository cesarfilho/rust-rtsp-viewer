//! Persistence for the user's view-mode choices.
//!
//! Runtime tweaks (grid density, carousel interval, camera order, active
//! group, layout, sidebar visibility) are written to
//! `$XDG_STATE_HOME/rust-rtsp-viewer/view.toml` (falling back to
//! `~/.local/state/...`) so they survive a restart. Everything is best-effort:
//! a missing or corrupt file just means "use the config.toml defaults", and a
//! failed write is logged and swallowed rather than interrupting the viewer.

use std::path::{Path, PathBuf};

use log::warn;
use serde::{Deserialize, Serialize};

/// On-disk mirror of the persisted subset of the view state. Every field is
/// optional so a file written by an older/newer build still loads, and so
/// `Default` means "inherit the config.toml value".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ViewStateFile {
    /// Grid density: `"auto"`, `"2x2"`, `"3x3"`, `"4x4"`, …
    pub mode: Option<String>,
    pub rotate_enabled: Option<bool>,
    pub rotate_secs: Option<u64>,
    /// Camera display order as a permutation of `0..n_cameras`.
    pub order: Option<Vec<usize>>,
    /// Index into `[[groups]]` of the active filter chip, `None` = "All".
    pub active_group: Option<usize>,
    /// `"grid"` or `"flex"`.
    pub layout: Option<String>,
    pub sidebar_visible: Option<bool>,
    /// Interface language (`"pt-BR"` / `"en"`), chosen in the menu.
    pub language: Option<String>,
}

/// `~/…` expansion limited to a leading `~/`, matching
/// `infrastructure::recording_paths`.
fn expand_tilde(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    path.to_path_buf()
}

/// Directory the state file lives in: `$XDG_STATE_HOME/rust-rtsp-viewer` when
/// that env var is set and absolute, else `~/.local/state/rust-rtsp-viewer`.
pub fn state_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_STATE_HOME") {
        let p = PathBuf::from(xdg);
        if p.is_absolute() {
            return p.join("rust-rtsp-viewer");
        }
    }
    expand_tilde(Path::new("~/.local/state/rust-rtsp-viewer"))
}

/// Full path to `view.toml`.
pub fn path() -> PathBuf {
    state_dir().join("view.toml")
}

/// Load the persisted view state. Returns `Default` (all `None`) when the file
/// is absent or unreadable; warns and returns `Default` when it is present but
/// cannot be parsed.
pub fn load() -> ViewStateFile {
    load_from(&path())
}

fn load_from(file: &Path) -> ViewStateFile {
    let raw = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return ViewStateFile::default(),
        Err(e) => {
            warn!("Could not read view state {}: {e}", file.display());
            return ViewStateFile::default();
        }
    };
    match toml::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            warn!("Ignoring malformed view state {}: {e}", file.display());
            ViewStateFile::default()
        }
    }
}

/// Persist the view state. Best-effort: directory-creation or write failures
/// are logged, never propagated. Writes to a temp file then renames so a
/// concurrent `load` never sees a half-written file.
pub fn save(state: &ViewStateFile) {
    save_to(&path(), state);
}

fn save_to(file: &Path, state: &ViewStateFile) {
    let body = match toml::to_string(state) {
        Ok(s) => s,
        Err(e) => {
            warn!("Could not serialize view state: {e}");
            return;
        }
    };
    if let Some(dir) = file.parent()
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        warn!("Could not create {}: {e}", dir.display());
        return;
    }
    let tmp = file.with_extension("toml.tmp");
    if let Err(e) = std::fs::write(&tmp, body.as_bytes()) {
        warn!("Could not write view state {}: {e}", tmp.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, file) {
        warn!("Could not finalise view state {}: {e}", file.display());
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "rrv-viewstate-{}-{}/view.toml",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn load_missing_file_is_default() {
        let p = tmp_path("missing");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        assert_eq!(load_from(&p), ViewStateFile::default());
    }

    #[test]
    fn save_then_load_round_trips() {
        let p = tmp_path("roundtrip");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        let state = ViewStateFile {
            mode: Some("3x3".into()),
            rotate_enabled: Some(true),
            rotate_secs: Some(15),
            order: Some(vec![2, 0, 1]),
            active_group: Some(1),
            layout: Some("grid".into()),
            sidebar_visible: Some(false),
        };
        save_to(&p, &state);
        assert_eq!(load_from(&p), state);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn malformed_file_falls_back_to_default() {
        let p = tmp_path("malformed");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"this is = not [valid toml").unwrap();
        assert_eq!(load_from(&p), ViewStateFile::default());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn partial_file_leaves_other_fields_none() {
        let p = tmp_path("partial");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"rotate_secs = 20\n").unwrap();
        let loaded = load_from(&p);
        assert_eq!(loaded.rotate_secs, Some(20));
        assert_eq!(loaded.mode, None);
        assert_eq!(loaded.order, None);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
