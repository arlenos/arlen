mod event_bus;
mod theme;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            theme::start_watcher(app.handle().clone());
            event_bus::start(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            theme::get_surface_tokens,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
