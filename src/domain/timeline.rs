use serde::Deserialize;

/// Types of events that can appear on the timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum EventType {
    /// Motion was detected.
    Motion,
    /// Recording started.
    RecordingStart,
    /// Recording stopped.
    RecordingStop,
    /// Camera went offline.
    Offline,
    /// Camera came back online.
    Online,
    /// Snapshot was taken.
    Snapshot,
}

impl EventType {
    pub fn label(&self) -> &'static str {
        match self {
            EventType::Motion => "Motion",
            EventType::RecordingStart => "Recording Start",
            EventType::RecordingStop => "Recording Stop",
            EventType::Offline => "Offline",
            EventType::Online => "Online",
            EventType::Snapshot => "Snapshot",
        }
    }

    pub fn color_hex(&self) -> &'static str {
        match self {
            EventType::Motion => "#FFA500",
            EventType::RecordingStart => "#FF0000",
            EventType::RecordingStop => "#808080",
            EventType::Offline => "#FF4444",
            EventType::Online => "#44FF44",
            EventType::Snapshot => "#4488FF",
        }
    }
}

/// A single event on the timeline.
#[derive(Debug, Clone)]
pub struct TimelineEvent {
    /// UNIX timestamp in seconds.
    pub timestamp_secs: u64,
    /// Camera index.
    pub camera_idx: usize,
    /// Type of event.
    pub event_type: EventType,
    /// Optional description.
    pub description: Option<String>,
}

impl TimelineEvent {
    pub fn new(timestamp_secs: u64, camera_idx: usize, event_type: EventType) -> Self {
        Self { timestamp_secs, camera_idx, event_type, description: None }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

/// Timeline configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct TimelineConfig {
    /// Maximum number of events to keep in memory.
    pub max_events: usize,
    /// Time window in seconds for the timeline view.
    pub window_secs: u64,
}

impl Default for TimelineConfig {
    fn default() -> Self {
        Self { max_events: 1000, window_secs: 3600 }
    }
}

/// TOML mirror for `[timeline]` section.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct TimelineConfigFile {
    pub max_events: Option<usize>,
    pub window_secs: Option<u64>,
}

impl TimelineConfigFile {
    pub fn into_config(self) -> TimelineConfig {
        let mut config = TimelineConfig::default();
        if let Some(m) = self.max_events { config.max_events = m.clamp(10, 100_000); }
        if let Some(w) = self.window_secs { config.window_secs = w.clamp(60, 86400); }
        config
    }
}

/// An in-memory event timeline for a single camera.
#[derive(Debug, Clone)]
pub struct EventTimeline {
    events: Vec<TimelineEvent>,
    max_events: usize,
}

impl EventTimeline {
    pub fn new(max_events: usize) -> Self {
        Self { events: Vec::new(), max_events }
    }

    /// Add an event to the timeline.
    pub fn push(&mut self, event: TimelineEvent) {
        self.events.push(event);
        // Trim oldest events if over limit.
        let excess = self.events.len().saturating_sub(self.max_events);
        if excess > 0 {
            self.events.drain(..excess);
        }
    }

    /// Get events within a time window (from_secs..to_secs).
    pub fn events_in_range(&self, from_secs: u64, to_secs: u64) -> Vec<&TimelineEvent> {
        self.events.iter()
            .filter(|e| e.timestamp_secs >= from_secs && e.timestamp_secs <= to_secs)
            .collect()
    }

    /// Get events for a specific camera.
    pub fn events_for_camera(&self, camera_idx: usize) -> Vec<&TimelineEvent> {
        self.events.iter()
            .filter(|e| e.camera_idx == camera_idx)
            .collect()
    }

    /// Total number of events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the timeline is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_type_labels() {
        assert_eq!(EventType::Motion.label(), "Motion");
        assert_eq!(EventType::RecordingStart.label(), "Recording Start");
        assert_eq!(EventType::Snapshot.label(), "Snapshot");
    }

    #[test]
    fn event_type_colors() {
        assert_eq!(EventType::Motion.color_hex(), "#FFA500");
        assert_eq!(EventType::Offline.color_hex(), "#FF4444");
    }

    #[test]
    fn timeline_push_and_len() {
        let mut t = EventTimeline::new(100);
        assert!(t.is_empty());
        t.push(TimelineEvent::new(1000, 0, EventType::Motion));
        t.push(TimelineEvent::new(1001, 0, EventType::Snapshot));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn timeline_trims_oldest() {
        let mut t = EventTimeline::new(3);
        t.push(TimelineEvent::new(1, 0, EventType::Motion));
        t.push(TimelineEvent::new(2, 0, EventType::Motion));
        t.push(TimelineEvent::new(3, 0, EventType::Motion));
        t.push(TimelineEvent::new(4, 0, EventType::Motion));
        assert_eq!(t.len(), 3);
        // Oldest event (timestamp 1) should be removed.
        let events = t.events_in_range(0, 5);
        assert!(events.iter().all(|e| e.timestamp_secs >= 2));
    }

    #[test]
    fn timeline_events_in_range() {
        let mut t = EventTimeline::new(100);
        t.push(TimelineEvent::new(100, 0, EventType::Motion));
        t.push(TimelineEvent::new(200, 0, EventType::Motion));
        t.push(TimelineEvent::new(300, 0, EventType::Motion));
        let range = t.events_in_range(150, 250);
        assert_eq!(range.len(), 1);
        assert_eq!(range[0].timestamp_secs, 200);
    }

    #[test]
    fn timeline_events_for_camera() {
        let mut t = EventTimeline::new(100);
        t.push(TimelineEvent::new(100, 0, EventType::Motion));
        t.push(TimelineEvent::new(100, 1, EventType::Motion));
        t.push(TimelineEvent::new(100, 0, EventType::Snapshot));
        assert_eq!(t.events_for_camera(0).len(), 2);
        assert_eq!(t.events_for_camera(1).len(), 1);
        assert_eq!(t.events_for_camera(2).len(), 0);
    }

    #[test]
    fn timeline_config_defaults() {
        let c = TimelineConfig::default();
        assert_eq!(c.max_events, 1000);
        assert_eq!(c.window_secs, 3600);
    }

    #[test]
    fn timeline_config_file_overrides() {
        let f = TimelineConfigFile { max_events: Some(500), window_secs: Some(7200) };
        let c = f.into_config();
        assert_eq!(c.max_events, 500);
        assert_eq!(c.window_secs, 7200);
    }

    #[test]
    fn timeline_config_file_clamps() {
        let f = TimelineConfigFile { max_events: Some(1), window_secs: Some(1) };
        let c = f.into_config();
        assert_eq!(c.max_events, 10);
        assert_eq!(c.window_secs, 60);
    }

    #[test]
    fn event_with_description() {
        let e = TimelineEvent::new(100, 0, EventType::Motion)
            .with_description("front yard");
        assert_eq!(e.description.as_deref(), Some("front yard"));
    }
}
