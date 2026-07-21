//! The pure Arlen settings core: config-write logic the Tauri host wraps as
//! commands, unit-tested in CI (the `src-tauri` host is not, its webkit deps
//! keep it off the runners).
//!
//! Today this is the format-preserving TOML writer. It updates only the keys the
//! user changed in a hand-authored config file, keeping every comment and blank
//! line, and writes atomically at 0600 (tmp + rename).
pub mod accessibility;
pub mod brightness;
pub mod config;
pub mod knowledge;
pub mod notifications;
pub mod toml_writer;
