//! Pure domain logic — no GTK, no GStreamer, no I/O.
//!
//! Everything in this module tree is a `#[cfg(test)]`-friendly pure
//! function or pure data structure. This is the layer that gets
//! exercised by `cargo test`. Infrastructure concerns (GTK widgets,
//! GStreamer pipelines, file I/O) live in `crate::infrastructure`.

pub mod audio;
pub mod bidirectional_audio;
pub mod camera_status;
pub mod codec;
pub mod diagnostics;
pub mod groups;
pub mod hw_encoder;
pub mod metrics;
pub mod motion;
pub mod multi_stream;
pub mod notify;
pub mod ptz;
pub mod recording;
pub mod redact;
pub mod retention;
pub mod snapshot;
pub mod streaming;
pub mod timelapse;
pub mod timeline;
pub mod timeline_view;
pub mod view;
pub mod zones;
