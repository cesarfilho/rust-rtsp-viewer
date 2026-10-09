//! Pure domain logic — no GTK, no GStreamer, no I/O.
//!
//! Everything in this module tree is a `#[cfg(test)]`-friendly pure
//! function or pure data structure. This is the layer that gets
//! exercised by `cargo test`. Infrastructure concerns (GTK widgets,
//! GStreamer pipelines, file I/O) live in `crate::infrastructure`.

pub mod audio;
pub mod camera_status;
pub mod codec;
pub mod detect;
pub mod detection_snapshot;
pub mod diagnostics;
pub mod groups;
pub mod metrics;
pub mod motion;
pub mod multi_stream;
pub mod notify;
pub mod preroll;
pub mod recent_frames;
pub mod recording;
pub mod redact;
pub mod retention;
pub mod snapshot;
pub mod static_objects;
pub mod timeline;
pub mod timeline_view;
pub mod view;
pub mod yuv;
pub mod zones;
