/// WaypointerPlugin trait — re-exported from module-sdk.
///
/// The trait + supporting types (SearchResult, Action, PluginDescriptor,
/// PluginError) live in `module_sdk::waypointer` so first-party and
/// third-party modules implement against the same definition. This file
/// preserves the existing import path
/// `crate::waypointer_system::plugin::*` so in-shell consumers (the 12
/// existing system plugins, PluginManager, registry.rs) need no changes.
///
/// When third-party Waypointer modules ship (Phase 4 sandbox), they
/// import from `module_sdk::waypointer` directly.

pub use module_sdk::waypointer::{
    Action, PluginDescriptor, PluginError, SearchResult, WaypointerPlugin,
};
