//! Which cameras have recording by detection on (`Engine::set_detect_armed`), kept across restarts
//! in `<state dir>/detection_armed.json` as a list of camera *names* (never index or URL), so a
//! reboot neither starts recording nobody asked for nor silently stops what the user turned on.

use std::path::{Path, PathBuf};

pub fn path() -> PathBuf {
    super::view_state::state_dir().join("detection_armed.json")
}

/// The cameras that were on; none without a file (it is switched on from the window).
pub fn load(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, cameras: &[String]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(cameras).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn none_without_a_file_and_the_choice_round_trips() {
        let dir = std::env::temp_dir().join(format!("rrv-armed-{}", std::process::id()));
        let p = dir.join("detection_armed.json");
        assert!(super::load(&p).is_empty());
        super::save(&p, &["Garagem".into()]).unwrap();
        assert_eq!(super::load(&p), ["Garagem"]);
        super::save(&p, &[]).unwrap();
        assert!(super::load(&p).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
