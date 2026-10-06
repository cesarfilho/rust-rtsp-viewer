//! The video engine and everything it needs, with no UI.
//!
//! This crate must never depend on `iced`: it is what the headless daemon
//! (ADR 0010) runs. The window (`rust-rtsp-viewer`) depends on it and re-exports
//! these modules so `crate::domain::…` paths keep working there.

pub mod config;
pub mod config_check;
pub mod domain;
pub mod engine;
pub mod i18n;
pub mod infrastructure;
pub mod ipc;
pub mod mqtt;
pub mod onvif;
pub mod secrets;
pub mod startup;
pub mod webhook;

/// Câmera simulada e utilitários dos testes (feature `testing`).
#[cfg(feature = "testing")]
pub mod testing;
