//! Persistence for per-camera motion zones.
//!
//! Zones drawn in the editor are written to
//! `$XDG_STATE_HOME/rust-rtsp-viewer/zones.toml`, keyed by the camera's display
//! name (never its URL, which carries credentials). Best-effort like
//! `view_state`: a missing or corrupt file means "no zones".

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use log::warn;
use serde::{Deserialize, Serialize};

use crate::domain::zones::{MotionZoneFile, ZoneConfig};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ZonesFile {
    /// Camera name → zones drawn for it.
    #[serde(default)]
    pub cameras: BTreeMap<String, Vec<MotionZoneFile>>,
}

impl ZonesFile {
    /// Zones stored for `camera`, empty when none.
    pub fn zone_config_for(&self, camera: &str) -> ZoneConfig {
        ZoneConfig {
            zones: self
                .cameras
                .get(camera)
                .map(|zs| zs.iter().cloned().map(MotionZoneFile::into_zone).collect())
                .unwrap_or_default(),
        }
    }

    /// Replace the stored zones for `camera`; an empty config removes the entry.
    pub fn set(&mut self, camera: &str, config: &ZoneConfig) {
        if config.zones.is_empty() {
            self.cameras.remove(camera);
        } else {
            self.cameras.insert(
                camera.to_string(),
                config.zones.iter().map(MotionZoneFile::from_zone).collect(),
            );
        }
    }
}

pub fn path() -> PathBuf {
    super::view_state::state_dir().join("zones.toml")
}

pub fn load() -> ZonesFile {
    load_from(&path())
}

pub fn save(file: &ZonesFile) {
    save_to(&path(), file);
}

fn load_from(file: &Path) -> ZonesFile {
    match std::fs::read_to_string(file) {
        Ok(raw) => toml::from_str(&raw).unwrap_or_else(|e| {
            warn!("Ignoring malformed zones file {}: {e}", file.display());
            ZonesFile::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ZonesFile::default(),
        Err(e) => {
            warn!("Could not read zones file {}: {e}", file.display());
            ZonesFile::default()
        }
    }
}

fn save_to(file: &Path, data: &ZonesFile) {
    let body = match toml::to_string(data) {
        Ok(s) => s,
        Err(e) => {
            warn!("Could not serialize zones: {e}");
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
        warn!("Could not write zones {}: {e}", tmp.display());
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, file) {
        warn!("Could not replace {}: {e}", file.display());
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::zones::{MotionZone, Point};

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("rrv-zones-{}-{}/zones.toml", std::process::id(), name))
    }

    fn square() -> ZoneConfig {
        ZoneConfig {
            zones: vec![MotionZone::new(
                "Entrada",
                vec![Point::new(0.1, 0.1), Point::new(0.9, 0.1), Point::new(0.5, 0.9)],
            )],
        }
    }

    #[test]
    fn missing_file_is_empty() {
        let p = tmp_path("missing");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        assert!(load_from(&p).cameras.is_empty());
    }

    #[test]
    fn round_trips_zones_by_camera_name() {
        let p = tmp_path("roundtrip");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
        let mut f = ZonesFile::default();
        f.set("Garagem", &square());
        save_to(&p, &f);
        let loaded = load_from(&p).zone_config_for("Garagem");
        assert_eq!(loaded.zones.len(), 1);
        assert_eq!(loaded.zones[0].name, "Entrada");
        assert_eq!(loaded.zones[0].vertices.len(), 3);
        assert!(load_from(&p).zone_config_for("Outra").zones.is_empty());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn empty_config_removes_entry() {
        let mut f = ZonesFile::default();
        f.set("Garagem", &square());
        f.set("Garagem", &ZoneConfig::default());
        assert!(f.cameras.is_empty());
    }

    #[test]
    fn malformed_file_falls_back_to_empty() {
        let p = tmp_path("malformed");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, b"not = [valid toml").unwrap();
        assert!(load_from(&p).cameras.is_empty());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
