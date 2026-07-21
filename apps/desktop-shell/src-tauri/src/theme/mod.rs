//! Theme system for Arlen (host side).
//!
//! The pure resolution pipeline (`schema`, `loader`, `css`) lives in the shell
//! core crate (`arlen_desktop_shell_core::theme`), unit-tested in CI. This module
//! holds the Tauri command layer + `start_appearance_watcher` that drive it.
//!
//! The legacy `SurfaceTokens` / `load_tokens` / `start_watcher` API was removed --
//! all theme data flows through `ThemeState` + `CssVariables` now. The single
//! event is `arlen://theme-v2-changed`.
pub mod commands;
