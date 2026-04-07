/// Built-in Waypointer plugins (Phase 2: compiled into shell).
///
/// Each plugin wraps existing functionality behind the WaypointerPlugin trait.

pub mod app_search;
pub mod calculator;
pub mod man;
pub mod process_kill;
pub mod shell;
pub mod unicode;
pub mod url;
pub mod window_switcher;

use super::manager::PluginManager;

/// Register all built-in plugins with the manager.
pub fn register_builtins(mgr: &mut PluginManager) {
    let plugins: Vec<Box<dyn super::plugin::WaypointerPlugin>> = vec![
        Box::new(app_search::AppSearchPlugin::new()),
        Box::new(url::UrlPlugin),
        Box::new(window_switcher::WindowSwitcherPlugin),
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
