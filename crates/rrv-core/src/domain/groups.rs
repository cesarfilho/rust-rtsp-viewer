use serde::Deserialize;

/// A named group of cameras, used for sidebar filtering
/// and layout switching.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CameraGroup {
    /// Display name for this group (e.g. "Front Yard").
    pub name: String,
    /// Indices into the `cameras` array (0-based).
    pub camera_indices: Vec<usize>,
}

impl CameraGroup {
    pub fn new(name: impl Into<String>, camera_indices: Vec<usize>) -> Self {
        Self {
            name: name.into(),
            camera_indices,
        }
    }

    /// Returns true if the given camera index belongs to this group.
    pub fn contains(&self, idx: usize) -> bool {
        self.camera_indices.contains(&idx)
    }

    /// Number of cameras in this group.
    pub fn len(&self) -> usize {
        self.camera_indices.len()
    }

    /// Whether the group is empty.
    pub fn is_empty(&self) -> bool {
        self.camera_indices.is_empty()
    }
}

/// TOML mirror for `[[groups]]` array.
#[derive(Debug, Deserialize, Clone)]
pub struct CameraGroupFile {
    pub name: String,
    pub cameras: Vec<usize>,
}

impl CameraGroupFile {
    pub fn into_group(self) -> CameraGroup {
        CameraGroup::new(self.name, self.cameras)
    }
}

/// Validate a list of groups against the total camera count.
/// Returns an error if any camera index is out of range or
/// if groups overlap.
pub fn validate_groups(groups: &[CameraGroup], total_cameras: usize) -> Result<(), String> {
    let mut seen = vec![false; total_cameras];
    for group in groups {
        for &idx in &group.camera_indices {
            if idx >= total_cameras {
                return Err(format!(
                    "Group '{}' references camera index {} but only {} cameras exist",
                    group.name, idx, total_cameras
                ));
            }
            if seen[idx] {
                return Err(format!("Camera index {} appears in multiple groups", idx));
            }
            seen[idx] = true;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_contains() {
        let g = CameraGroup::new("Front", vec![0, 2, 4]);
        assert!(g.contains(0));
        assert!(!g.contains(1));
        assert!(g.contains(4));
    }

    #[test]
    fn group_len_and_is_empty() {
        let empty = CameraGroup::new("Empty", vec![]);
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let g = CameraGroup::new("A", vec![0, 1]);
        assert!(!g.is_empty());
        assert_eq!(g.len(), 2);
    }

    #[test]
    fn validate_groups_ok() {
        let groups = vec![
            CameraGroup::new("A", vec![0, 1]),
            CameraGroup::new("B", vec![2, 3]),
        ];
        assert!(validate_groups(&groups, 4).is_ok());
    }

    #[test]
    fn validate_groups_out_of_range() {
        let groups = vec![CameraGroup::new("A", vec![0, 5])];
        assert!(validate_groups(&groups, 3).is_err());
    }

    #[test]
    fn validate_groups_overlap() {
        let groups = vec![
            CameraGroup::new("A", vec![0, 1]),
            CameraGroup::new("B", vec![1, 2]),
        ];
        assert!(validate_groups(&groups, 3).is_err());
    }

    #[test]
    fn validate_groups_empty_list() {
        assert!(validate_groups(&[], 5).is_ok());
    }

    #[test]
    fn group_file_conversion() {
        let f = CameraGroupFile {
            name: "Yard".into(),
            cameras: vec![0, 2],
        };
        let g = f.into_group();
        assert_eq!(g.name, "Yard");
        assert_eq!(g.camera_indices, vec![0, 2]);
    }
}
