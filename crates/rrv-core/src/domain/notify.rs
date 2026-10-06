//! Desktop-notification policy (pure): which events notify and how often.

use crate::domain::timeline::EventType;

pub const DEFAULT_COOLDOWN_SECS: u64 = 60;
pub const MIN_COOLDOWN_SECS: u64 = 5;

#[derive(Debug, Clone)]
pub struct NotifyConfig {
    pub enabled: bool,
    /// Minimum seconds between two notifications of the same kind for the same camera.
    pub cooldown_secs: u64,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cooldown_secs: DEFAULT_COOLDOWN_SECS,
        }
    }
}

/// Title/body for an event worth interrupting the user for; `None` for the rest
/// (recording start/stop and snapshots are the user's own doing).
pub fn message_for(
    kind: EventType,
    camera: &str,
    detail: Option<&str>,
) -> Option<(String, String)> {
    let (title, body) = match kind {
        EventType::Motion => ("Movimento detectado", detail.unwrap_or("")),
        EventType::Offline => ("Câmera offline", "Sem sinal de vídeo"),
        _ => return None,
    };
    let body = if body.is_empty() {
        camera.to_string()
    } else {
        format!("{camera} · {body}")
    };
    Some((title.to_string(), body))
}

/// Title/body of a "something was recognised" notification.
pub fn detection_message(
    label: &str,
    camera: &str,
    percent: f32,
    zone: Option<&str>,
) -> (String, String) {
    let title = format!("Detecção: {}", crate::domain::detect::label_pt(label));
    let body = match zone {
        Some(z) => format!("{camera} · {percent}% · {z}"),
        None => format!("{camera} · {percent}%"),
    };
    (title, body)
}

/// Whether enough time has passed since the last notification of this kind.
pub fn cooldown_elapsed(secs_since_last: Option<u64>, cooldown_secs: u64) -> bool {
    secs_since_last.is_none_or(|s| s >= cooldown_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_motion_and_offline_notify() {
        assert!(message_for(EventType::Motion, "Garagem", Some("3.2% do quadro")).is_some());
        assert!(message_for(EventType::Offline, "Garagem", None).is_some());
        assert!(message_for(EventType::Snapshot, "Garagem", None).is_none());
        assert!(message_for(EventType::RecordingStart, "Garagem", None).is_none());
    }

    #[test]
    fn detection_messages_name_the_object_the_camera_and_the_zone() {
        let (t, b) = detection_message("person", "Garagem", 87.0, Some("Portão"));
        assert_eq!(t, "Detecção: pessoa");
        assert_eq!(b, "Garagem · 87% · Portão");
        assert_eq!(
            detection_message("kite", "G", 50.0, None).0,
            "Detecção: kite"
        );
    }

    #[test]
    fn body_names_the_camera() {
        let (_, body) = message_for(EventType::Offline, "Garagem", None).unwrap();
        assert!(body.starts_with("Garagem"));
    }

    #[test]
    fn cooldown_gate() {
        assert!(cooldown_elapsed(None, 60));
        assert!(!cooldown_elapsed(Some(59), 60));
        assert!(cooldown_elapsed(Some(60), 60));
    }
}
