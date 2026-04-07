/// Built-in Waypointer plugins (Phase 2: compiled into shell).

pub mod app_search;
pub mod calculator;
pub mod man;
pub mod process_kill;
pub mod shell;
pub mod unicode;
pub mod url;
pub mod window_switcher;

use crate::app_index;
use crate::wayland_client;
use super::manager::PluginManager;

/// Register all built-in plugins with the manager.
///
/// Plugins that need shared state receive cloned Arc references.
pub fn register_builtins(
    mgr: &mut PluginManager,
    app_index: app_index::AppIndex,
    window_list: wayland_client::WindowList,
) {
    let plugins: Vec<Box<dyn super::plugin::WaypointerPlugin>> = vec![
        Box::new(app_search::AppSearchPlugin::new(app_index)),
        Box::new(url::UrlPlugin),
        Box::new(window_switcher::WindowSwitcherPlugin::new(window_list)),
        Box::new(calculator::CalculatorPlugin),
        Box::new(shell::ShellPlugin),
        Box::new(man::ManPlugin),
        Box::new(process_kill::ProcessKillPlugin),
        Box::new(unicode::UnicodePlugin),
    ];

    for plugin in plugins {
        if let Err(e) = mgr.register(plugin) {
            log::warn!("failed to register built-in plugin: {e}");
        }
    }
}
