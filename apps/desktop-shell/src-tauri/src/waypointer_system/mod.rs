/// Waypointer plugin system (Phase 2: internal compiled plugins).
///
/// Defines the `WaypointerPlugin` trait and `PluginManager` that aggregates
/// results from all registered plugins, sorted by relevance and priority.
///
/// See `docs/architecture/waypointer-migration.md`.

mod plugin;
mod manager;

pub use plugin::*;
pub use manager::*;
