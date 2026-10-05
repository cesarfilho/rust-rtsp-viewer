use std::time::Instant;

pub const TOAST_DURATION_SECS: u64 = 3;
pub const VU_PEAK_DECAY_MS: u128 = 1200;

pub struct Toast {
    pub message: String,
    pub shown_at: Instant,
    /// Folder a click on the toast opens (snapshot saved, …).
    pub open_dir: Option<std::path::PathBuf>,
}

pub struct ContextMenu {
    pub camera_idx: usize,
    /// Where the menu opened. Captured once: reading the live pointer position
    /// at render time made the menu follow the mouse.
    pub anchor: iced::Point,
}

pub fn is_expired(toast: &Toast) -> bool {
    // Clickable toasts stay up longer so there is time to aim at them.
    let ttl = if toast.open_dir.is_some() {
        TOAST_DURATION_SECS * 2
    } else {
        TOAST_DURATION_SECS
    };
    toast.shown_at.elapsed().as_secs() >= ttl
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_not_expired_initially() {
        let t = Toast {
            message: "test".into(),
            shown_at: Instant::now(),
            open_dir: None,
        };
        assert!(!is_expired(&t));
    }

    #[test]
    fn toast_expires_after_duration() {
        let t = Toast {
            message: "test".into(),
            shown_at: Instant::now() - std::time::Duration::from_secs(TOAST_DURATION_SECS + 1),
            open_dir: None,
        };

        assert!(is_expired(&t));

        let clickable = Toast {
            open_dir: Some("/tmp".into()),
            ..t
        };
        assert!(!is_expired(&clickable));
    }

    #[test]
    fn context_menu_holds_camera_idx() {
        let cm = ContextMenu {
            camera_idx: 3,
            anchor: iced::Point::new(10.0, 20.0),
        };
        assert_eq!(cm.camera_idx, 3);
        assert_eq!(cm.anchor, iced::Point::new(10.0, 20.0));
    }
}
