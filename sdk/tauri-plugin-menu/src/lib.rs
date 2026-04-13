/// Tauri plugin for the Lunaris titlebar protocol.
///
/// Connects to the compositor as a Wayland client, binds the
/// `lunaris-titlebar-v1` protocol, and exposes titlebar management
/// (tabs, buttons, breadcrumbs, search) to the Tauri frontend via
/// commands and events.
///
/// # Usage (Rust)
///
/// ```rust,ignore
/// fn main() {
///     tauri::Builder::default()
///         .plugin(tauri_plugin_lunaris_menu::init())
///         .run(tauri::generate_context!())
///         .expect("error running app");
/// }
/// ```
///
/// # Usage (TypeScript)
///
/// ```typescript
/// import { addTab, onTabActivated } from '@lunaris/tauri-plugin-menu';
///
/// addTab({ id: "1", title: "main.rs", status: "modified" });
/// onTabActivated((id) => console.log("activated:", id));
/// ```

mod commands;
mod protocol;
mod wayland;

use std::sync::{Arc, Mutex};

use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

use wayland::{SharedConnection, TitlebarConnection};

/// Initialize the Lunaris menu plugin.
///
/// Registers all titlebar commands and starts the Wayland client thread
/// that connects to the compositor's `lunaris-titlebar-v1` protocol.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("lunaris-menu")
        .invoke_handler(tauri::generate_handler![
            commands::set_title,
            commands::set_breadcrumb,
            commands::set_center_content,
            commands::add_tab,
            commands::remove_tab,
            commands::update_tab,
            commands::activate_tab,
            commands::reorder_tabs,
            commands::add_button,
            commands::remove_button,
            commands::set_button_enabled,
            commands::set_search_mode,
        ])
        .setup(|app, _api| {
            let shared: SharedConnection =
                Arc::new(Mutex::new(TitlebarConnection::default()));
            app.manage(shared.clone());
            wayland::start(app.clone(), shared);
            Ok(())
        })
        .build()
}
