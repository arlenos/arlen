/// Theme system for Lunaris.
///
/// `schema` defines the full token hierarchy. `loader` handles resolution
/// from built-in TOML files, user overrides, and accessibility settings.
/// `commands` provides Tauri commands and the `start_appearance_watcher`.
/// `css` generates injectable CSS variable strings.
///
/// The legacy `SurfaceTokens` / `load_tokens` / `start_watcher` API was
/// removed -- all theme data flows through `ThemeState` + `CssVariables`
/// now. The single event is `lunaris://theme-v2-changed`.

pub mod commands;
pub mod css;
pub mod loader;
pub mod schema;
