/// Built-in Waypointer plugins (Phase 2: compiled into shell).

pub mod app_search;
pub mod calculator;
pub mod clipboard;
pub mod datetime;
pub mod dict;
pub mod files;
pub mod man;
pub mod power;
pub mod process_kill;
pub mod quick_actions;
pub mod projects;
pub mod shell;
pub mod unicode;
pub mod unit_converter;
pub mod url;
pub mod window_switcher;

use crate::app_index;
use crate::clipboard_history::ClipboardHistoryState;
use crate::wayland_client;
use super::manager::PluginManager;
use super::registry;

/// Register all built-in plugins with the manager.
///
/// Plugins listed in `~/.config/lunaris/modules.toml` under
/// `[waypointer] disabled_plugins` are skipped entirely (no trait
/// methods called, not even `search` — so they don't contribute to
/// results and don't cost anything at runtime). After registration
/// the current roster is written to
/// `~/.local/share/lunaris/waypointer-plugins.toml` so the Settings
/// app can display it.
///
/// Plugins that need shared state receive cloned Arc references.
pub fn register_builtins(
    mgr: &mut PluginManager,
    app_index: app_index::AppIndex,
    window_list: wayland_client::WindowList,
    clipboard: ClipboardHistoryState,
) {
    let disabled = registry::load_disabled_plugins();
    if !disabled.is_empty() {
        log::info!("waypointer: disabled plugins from config: {disabled:?}");
    }

    let plugins: Vec<Box<dyn super::plugin::WaypointerPlugin>> = vec![
        Box::new(app_search::AppSearchPlugin::new(app_index)),
        Box::new(url::UrlPlugin),
        Box::new(window_switcher::WindowSwitcherPlugin::new(window_list)),
        Box::new(calculator::CalculatorPlugin),
        Box::new(unit_converter::UnitConverterPlugin),
        Box::new(datetime::DateTimePlugin),
        Box::new(projects::ProjectsPlugin),
        Box::new(files::FilesPlugin::new()),
        Box::new(clipboard::ClipboardPlugin::new(clipboard)),
        Box::new(dict::DictPlugin::new()),
        Box::new(power::PowerPlugin),
        Box::new(quick_actions::QuickActionsPlugin),
        Box::new(shell::ShellPlugin),
        Box::new(man::ManPlugin),
        Box::new(process_kill::ProcessKillPlugin),
        Box::new(unicode::UnicodePlugin),
    ];

    for plugin in plugins {
        if disabled.iter().any(|d| d == plugin.id()) {
            log::info!("waypointer: skipping disabled plugin '{}'", plugin.id());
            continue;
        }
        if let Err(e) = mgr.register(plugin) {
            log::warn!("failed to register built-in plugin: {e}");
        }
    }

    registry::write_registry(mgr);
}
