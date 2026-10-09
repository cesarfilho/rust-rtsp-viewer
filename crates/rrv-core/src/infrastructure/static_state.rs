//! The fixed spots the detection learned (`domain::static_objects`), kept across restarts in
//! `<state dir>/static_spots.json`, by camera *name* (never index or URL, like the zones).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::static_objects::StaticSpot;

pub type StaticSpots = HashMap<String, Vec<StaticSpot>>;

pub fn path() -> PathBuf {
    super::view_state::state_dir().join("static_spots.json")
}

/// What was saved; empty when there is no file or it cannot be read (it is only a cache: the
/// spots are learned again).
pub fn load(path: &Path) -> StaticSpots {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Writes through a `.tmp` and a rename, so a crash never leaves half a file.
pub fn save(path: &Path, spots: &StaticSpots) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let json = serde_json::to_string_pretty(spots).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_round_trip_and_a_missing_or_broken_file_is_empty() {
        let dir = std::env::temp_dir().join(format!("rrv-static-{}", std::process::id()));
        let p = dir.join("static_spots.json");
        assert!(load(&p).is_empty());
        let mut spots = StaticSpots::new();
        spots.insert(
            "Garagem".into(),
            vec![StaticSpot {
                class: 0,
                x: 0.69,
                y: 0.72,
                w: 0.03,
                h: 0.11,
                last_seen: 1,
            }],
        );
        save(&p, &spots).unwrap();
        assert_eq!(load(&p), spots);
        std::fs::write(&p, "{ quebrado").unwrap();
        assert!(load(&p).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
