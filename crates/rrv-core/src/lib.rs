//! The video engine and everything it needs, with no UI.
//!
//! This crate must never depend on `iced`: it is what the headless daemon
//! (ADR 0010) runs. The window (`rust-rtsp-viewer`) depends on it and re-exports
//! these modules so `crate::domain::…` paths keep working there.

pub mod config;
pub mod config_check;
pub mod domain;
pub mod engine;
pub mod infrastructure;
