//! Whether the detection is armed (`Engine::set_detect_armed`), kept across restarts in
//! `<state dir>/detection_armed` (`1` or `0`), so a reboot neither starts recording nobody asked
//! for nor silently stops what the user started from the window.

use std::path::{Path, PathBuf};

pub fn path() -> PathBuf {
    super::view_state::state_dir().join("detection_armed")
}

/// What was saved; disarmed without a file (the detection is started from the window).
pub fn load(path: &Path) -> bool {
    std::fs::read_to_string(path).is_ok_and(|s| s.trim() == "1")
}

pub fn save(path: &Path, armed: bool) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::write(path, if armed { "1\n" } else { "0\n" })
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn disarmed_without_a_file_and_the_choice_round_trips() {
        let dir = std::env::temp_dir().join(format!("rrv-armed-{}", std::process::id()));
        let p = dir.join("detection_armed");
        assert!(!super::load(&p));
        super::save(&p, true).unwrap();
        assert!(super::load(&p));
        super::save(&p, false).unwrap();
        assert!(!super::load(&p));
        let _ = std::fs::remove_dir_all(dir);
    }
}
