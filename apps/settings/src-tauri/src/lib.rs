//! Lunaris Settings App backend.
//!
//! Tauri entry point. Registers the managed state for the config watcher
//! and wires up all command handlers.

mod commands;
mod config_watcher;

/// Tauri application entry point invoked from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_lunaris_menu::init())
        .setup(|app| {
            // Spawn the multi-file config watcher. It emits
            // `config:{file}:changed` Tauri events to the frontend.
            config_watcher::start_appearance_watcher(app.handle().clone());

            // Parse CLI arguments and stash them so the frontend can
            // pull them via `get_launch_args()` after mount. This
            // avoids the race where an event fires before the webview
            // is ready.
            commands::search::store_launch_args();

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::config::config_get,
            commands::config::config_set,
            commands::config::config_reset,
            commands::config::config_get_default,
            commands::theme::theme_get,
            commands::theme::theme_set_mode,
            commands::theme::theme_set_accent,
            commands::notifications::notifications_get_known_apps,
            commands::notifications::notifications_clear_history,
            commands::notifications::notifications_test_notification,
            commands::notifications::notifications_dnd_expiry_in,
            commands::notifications::notifications_dnd_expiry_until_morning,
            commands::modules::modules_list,
            commands::modules::modules_set_enabled,
            commands::modules::modules_uninstall,
            commands::search::export_settings_index,
            commands::search::get_launch_args,
            commands::input::keybindings_get_all,
            commands::input::keybindings_set,
            commands::input::keybindings_add_custom,
            commands::input::keybindings_remove,
            commands::input::keybindings_get_conflicts,
            commands::input::keybindings_get_defaults,
            commands::input::keybindings_reset_all,
            commands::input::keybindings_reset_all_to_defaults,
            commands::input::keybindings_reset_module_fragments,
            commands::input::keybindings_query_live_conflicts,
            commands::input::keybindings_get_all_conflicts,
            commands::input::mouse_get_config,
            commands::input::mouse_set_config,
            commands::input::touchpad_get_config,
            commands::input::touchpad_set_config,
            commands::input::keyboard_get_layouts,
            commands::input::keyboard_set_layouts,
            commands::input::keyboard_get_variants,
            commands::input::keyboard_set_variants,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
