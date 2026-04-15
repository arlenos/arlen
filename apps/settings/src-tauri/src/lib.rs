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
            // Spawn the appearance.toml file watcher. It emits
            // `config:appearance:changed` Tauri events to the frontend.
            config_watcher::start_appearance_watcher(app.handle().clone());
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
