//! The download engine of Tachy Download.
//!
//! It knows nothing about windows or Tauri: the app crate (`src-tauri`) is a shell around it.
//! The behaviour to reproduce is that of the JavaScript engine in `src/engine/` (see the design document
//! `docs/superpowers/specs/2026-11-04-rust-tauri-rewrite-design.md`).

/// Name shown in titles, notifications and the tray tooltip.
pub const APP_NAME: &str = "Tachy Download";

pub mod afterall;
pub mod ctx;
pub mod errors;
pub mod filetypes;
pub mod fsutil;
pub mod limiter;
pub mod logins;
pub mod manager;
pub mod menu;
pub mod net;
pub mod postprocess;
pub mod probe;
pub mod progress;
pub mod rules;
pub mod server;
pub mod settings;
pub mod state;
pub mod store;
pub mod task;
pub mod tools;
pub mod updater;
pub mod util;
pub mod video;
