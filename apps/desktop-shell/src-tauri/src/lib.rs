mod app_index;
mod audio;
mod battery;
mod event_bus;
mod gtk_menu_bridge;
mod layer_shell;
mod menu_store;
mod modules;
mod module_errors;
mod extension_registry;
mod bluetooth;
mod network;
mod shell_config;
mod power;
mod notifications;
mod permissions;
mod sni;
mod shell_overlay_client;
mod shell_runner;
mod theme;
mod wayland_client;
mod waypointer;
mod waypointer_plugins;
mod waypointer_processes;
mod waypointer_unicode;

use std::collections::HashMap;
use std::sync::Arc;
use tauri::Manager;

/// Relay a log message from the frontend to the terminal.
#[tauri::command]
fn log_frontend(message: String) {
    println!("[FRONTEND] {message}");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    // Created before Builder so they can be both managed and moved into start().
    let overlay_sender = Arc::new(shell_overlay_client::ShellOverlaySender::new());
    let workspace_sender = Arc::new(wayland_client::WorkspaceSender::new());
    let toplevel_sender = Arc::new(wayland_client::ToplevelSender::new());
    let window_list: wayland_client::WindowList = Arc::new(std::sync::Mutex::new(Vec::new()));
    let menu_store: menu_store::AppMenuStore =
        Arc::new(std::sync::Mutex::new(HashMap::new()));
    let menu_store_for_bridge = Arc::clone(&menu_store);
    let app_idx: app_index::AppIndex = Arc::new(std::sync::Mutex::new(app_index::build_index()));
    let sni_items: sni::SniItems = Arc::new(std::sync::Mutex::new(HashMap::new()));
    let module_loader: modules::ModuleLoaderState = std::sync::Mutex::new(modules::ModuleLoader::new());
    let error_tracker: module_errors::ErrorTrackerState = std::sync::Mutex::new(module_errors::ModuleErrorTracker::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::clone(&overlay_sender))
        .manage(Arc::clone(&workspace_sender))
        .manage(Arc::clone(&toplevel_sender))
        .manage(Arc::clone(&window_list))
        .manage(Arc::clone(&menu_store))
        .manage(app_idx)
        .manage(Arc::clone(&sni_items))
        .manage(module_loader)
        .manage(error_tracker)
        .setup(|app| {
            // Initialize the new theme system (v2).
            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join("lunaris");
            let data_dir = dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join("lunaris");
            match theme::commands::ThemeState::new(config_dir, data_dir) {
                Ok(ts) => { app.manage(ts); }
                Err(e) => {
                    log::warn!("theme: state init failed ({e}), using in-memory defaults");
                    let fallback = theme::commands::ThemeState::new(
                        std::path::PathBuf::from("/tmp/lunaris-fallback"),
                        std::path::PathBuf::from("/tmp/lunaris-fallback"),
                    ).unwrap();
                    app.manage(fallback);
                }
            }

            theme::start_watcher(app.handle().clone());
            event_bus::start(app.handle().clone());
            wayland_client::start(app.handle().clone(), workspace_sender, toplevel_sender, window_list);
            shell_overlay_client::start(app.handle().clone(), overlay_sender);
            notifications::start(app.handle().clone());
            sni::start(app.handle().clone(), sni_items);
            bluetooth::start_monitor(app.handle().clone());
            gtk_menu_bridge::start(app.handle().clone(), menu_store_for_bridge);

            // Create the Waypointer overlay window (hidden).
            if let Err(e) = waypointer::create_window(app.handle()) {
                log::error!("waypointer: window creation failed: {e}");
            }

            #[cfg(target_os = "linux")]
            {
                let window_clone = app.get_webview_window("main").unwrap();
                let wp_clone = app.get_webview_window("waypointer");
                glib::idle_add_once(move || {
                    if let Err(e) = layer_shell::init(window_clone) {
                        log::error!("layer_shell: init failed: {e}");
                    }
                    if let Some(wp) = wp_clone {
                        waypointer::init_layer_shell(wp);
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            log_frontend,
            theme::get_surface_tokens,
            shell_overlay_client::context_menu_activate,
            shell_overlay_client::context_menu_dismiss,
            shell_overlay_client::tab_activate,
            shell_overlay_client::zoom_increase,
            shell_overlay_client::zoom_decrease,
            shell_overlay_client::zoom_close,
            shell_overlay_client::zoom_set_increment,
            shell_overlay_client::zoom_set_movement,
            shell_overlay_client::window_header_action,
            shell_overlay_client::set_notification_input_region,
            shell_overlay_client::set_popover_input_region,
            shell_overlay_client::resolve_app_icon,
            shell_overlay_client::debug_workspace_update,
            menu_store::register_menu,
            menu_store::unregister_menu,
            menu_store::dispatch_menu_action,
            menu_store::get_menu,
            waypointer::toggle_waypointer,
            audio::get_audio_status,
            audio::set_audio_volume,
            audio::toggle_audio_mute,
            audio::get_audio_outputs,
            audio::set_audio_output,
            audio::get_audio_inputs,
            audio::set_audio_input,
            battery::get_battery_status,
            power::get_power_profile,
            power::set_power_profile,
            network::get_network_status,
            network::get_wifi_networks,
            network::connect_wifi,
            network::connect_wifi_password,
            network::disconnect_wifi,
            network::get_wifi_enabled,
            network::set_wifi_enabled,
            network::get_airplane_mode,
            network::set_airplane_mode,
            bluetooth::get_bluetooth_state,
            bluetooth::set_bluetooth_powered,
            bluetooth::connect_bluetooth_device,
            bluetooth::disconnect_bluetooth_device,
            bluetooth::remove_bluetooth_device,
            bluetooth::set_device_trusted,
            bluetooth::start_bluetooth_scan,
            bluetooth::stop_bluetooth_scan,
            bluetooth::pair_bluetooth_device,
            shell_config::get_shell_config,
            shell_config::save_shell_config,
            permissions::get_app_permissions,
            permissions::get_app_permission_detail,
            modules::list_modules,
            modules::set_module_enabled,
            module_errors::record_module_error,
            module_errors::get_module_errors,
            module_errors::reset_module_errors,
            theme::commands::get_theme,
            theme::commands::get_theme_css,
            theme::commands::set_theme,
            theme::commands::get_available_themes,
            theme::commands::get_active_theme_id,
            theme::commands::set_accent_color,
            theme::commands::set_font_scale,
            theme::commands::set_reduce_motion,
            theme::commands::get_appearance_config,
            theme::commands::reset_theme,
            sni::get_sni_items,
            sni::activate_sni_item,
            sni::get_sni_menu,
            sni::click_sni_menu_item,
            shell_runner::execute_shell_command,
            shell_runner::open_url,
            app_index::get_apps,
            app_index::search_apps,
            app_index::launch_app,
            waypointer_plugins::evaluate_waypointer_input,
            wayland_client::workspace_activate,
            wayland_client::activate_window,
            wayland_client::get_windows,
            waypointer_processes::get_processes,
            waypointer_processes::kill_process,
            waypointer_unicode::search_unicode,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
