//! The pure Arlen desktop-shell core: shell logic the Tauri host wraps as
//! commands, unit-tested in CI (the `src-tauri` host is not, its webkit deps keep
//! it off the runners).
//!
//! Today this is the theme system: the token schema, resolution from built-in
//! TOML + user overrides + accessibility settings, and CSS-variable generation.
//! The Tauri command layer + the appearance watcher stay in the host.
pub mod modulesd_client;
pub mod notifications;
pub mod theme;
