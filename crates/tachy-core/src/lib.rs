//! The download engine of Tachy Download.
//!
//! It knows nothing about windows or Tauri: the app crate (`src-tauri`) is a shell around it.
//! The behaviour to reproduce is that of the JavaScript engine in `src/engine/` (see the design document
//! `docs/superpowers/specs/2026-11-04-rust-tauri-rewrite-design.md`).

/// Name shown in titles, notifications and the tray tooltip.
pub const APP_NAME: &str = "Tachy Download";
