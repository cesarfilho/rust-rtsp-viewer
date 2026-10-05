//! The window. The video engine lives in `rrv-core`; its modules are re-exported
//! here so `crate::domain::…`, `crate::engine::…` and friends keep working.

pub use rrv_core::{config, config_check, domain, engine, infrastructure, ipc, startup};

pub mod ui;
