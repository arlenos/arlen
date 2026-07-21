//! Theme system for Arlen: the pure resolution pipeline.
//!
//! `schema` defines the full token hierarchy (re-exporting the canonical
//! `arlen-theme` types). `loader` resolves a theme from built-in TOML files, user
//! overrides and accessibility settings. `css` generates the injectable CSS
//! variable strings. The Tauri commands + `start_appearance_watcher` live in the
//! shell host (`crate::theme::commands` there), not here.
pub mod css;
pub mod loader;
pub mod schema;
