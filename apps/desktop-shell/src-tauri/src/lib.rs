mod event_bus;
mod layer_shell;
mod notifications;
mod shell_overlay_client;
mod theme;
mod wayland_client;

use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    // Created before Builder so they can be both managed and moved into start().
    let overlay_sender = Arc::new(shell_overlay_client::ShellOverlaySender::new());
    let workspace_sender = Arc::new(wayland_client::WorkspaceSender::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::clone(&overlay_sender))
        .manage(Arc::clone(&workspace_sender))
        .setup(|app| {
            theme::start_watcher(app.handle().clone());
            event_bus::start(app.handle().clone());
            wayland_client::start(app.handle().clone(), workspace_sender);
            shell_overlay_client::start(app.handle().clone(), overlay_sender);
            notifications::start(app.handle().clone());

            #[cfg(target_os = "linux")]
            {
                let window_clone = app.get_webview_window("main").unwrap();
                glib::idle_add_once(move || {
                    if let Err(e) = layer_shell::init(window_clone) {
                        log::error!("layer_shell: init failed: {e}");
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            wayland_client::workspace_activate,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
