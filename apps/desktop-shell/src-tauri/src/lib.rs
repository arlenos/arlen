mod event_bus;
mod shell_overlay_client;
mod theme;
mod wayland_client;

use std::sync::Arc;

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
