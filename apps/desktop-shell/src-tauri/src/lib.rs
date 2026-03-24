mod event_bus;
mod layer_shell;
mod shell_overlay_client;
mod theme;
mod wayland_client;

use std::sync::Arc;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();

    // Created before Builder so it can be both managed and moved into start().
    let overlay_sender = Arc::new(shell_overlay_client::ShellOverlaySender::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Arc::clone(&overlay_sender))
        .setup(|app| {
            theme::start_watcher(app.handle().clone());
            event_bus::start(app.handle().clone());
            wayland_client::start(app.handle().clone());
            shell_overlay_client::start(app.handle().clone(), overlay_sender);

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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
