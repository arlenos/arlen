//! Lunaris Settings App backend.
//!
//! Tauri entry point. Registers the managed state for the config watcher
//! and wires up all command handlers.

mod commands;
mod config_watcher;
mod displays;

/// Tauri application entry point invoked from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // env_logger's default filter is `error`, which silently drops
    // every `log::info!` from the wayland output-management thread.
    // Default to `info` so the Display panel is debuggable; users
    // can still override via `RUST_LOG=…`.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

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

            // Spawn the wlr-output-management Wayland client on a
            // dedicated thread. Failure is non-fatal: under non-
            // Lunaris compositors the protocol may be missing and
            // the Display panel just shows an empty list. Settings
            // still launches.
            match displays::wayland_client::spawn(app.handle().clone()) {
                Ok(handle) => {
                    use tauri::Manager;
                    app.manage(std::sync::Arc::new(handle));
                }
                Err(err) => {
                    log::warn!(
                        "displays: wayland output-management unavailable, panel will be empty: {err}",
                    );
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::frontend_log,
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
            commands::waypointer_plugins::waypointer_list_plugins,
            commands::waypointer_plugins::waypointer_set_plugin_enabled,
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
            commands::displays::display_get_monitors,
            commands::displays::display_apply_config,
            commands::displays::display_revert,
            commands::displays::display_save_current,
            commands::displays::display_profiles_list,
            commands::displays::display_profile_save,
            commands::displays::display_profile_apply,
            commands::displays::display_profile_delete,
            commands::displays::display_profile_rename,
            commands::night_light::night_light_get_state,
            commands::night_light::night_light_set,
            commands::night_light::night_light_set_schedule,
            commands::night_light::night_light_set_location,
            commands::brightness::brightness_get_devices,
            commands::brightness::brightness_set,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
