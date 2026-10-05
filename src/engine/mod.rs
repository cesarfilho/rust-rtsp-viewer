//! The video engine: everything that keeps cameras running, with no UI.
//!
//! Nothing in here may depend on `iced` or import `crate::ui` — it is meant to
//! move into a headless daemon (ADR 0010). `tests/engine_isolation.rs` enforces
//! it. The UI reaches the engine through [`crate::ui`]'s re-exports.

pub mod backoff;
pub mod bridge;
pub mod pipeline;
