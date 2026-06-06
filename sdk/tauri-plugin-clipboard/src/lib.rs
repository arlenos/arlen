//! Tauri plugin for the Lunaris clipboard subsystem.
//!
//! Wraps [`os_sdk::UnixClipboardClient`] and exposes
//! `write` / `read` / `history` / `subscribe` / `unsubscribe`
//! Tauri commands plus a `lunaris://clipboard-changed` event
//! that the frontend can listen to for live updates.
//!
//! # Public API
//!
//! See [`commands`] for the Tauri commands and [`types`] for
//! the frontend-facing data types.
//!
//! # Usage (Rust)
//!
//! ```rust,ignore
//! fn main() {
//!     tauri::Builder::default()
//!         .plugin(tauri_plugin_lunaris_clipboard::init())
//!         .run(tauri::generate_context!())
//!         .expect("error running app");
//! }
//! ```

mod commands;
pub mod types;

pub use types::{ClipboardEntry, ClipboardError, ClipboardLabel, WriteParams};

use tauri::{
    plugin::{Builder, TauriPlugin},
    Manager, Runtime,
};

/// Initialise the Lunaris clipboard plugin. Registers all five
/// Tauri commands and the per-app `ClipboardState`.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("lunaris-clipboard")
        .invoke_handler(tauri::generate_handler![
            commands::write,
            commands::read,
            commands::history,
            commands::subscribe,
            commands::unsubscribe,
        ])
        .setup(|app, _api| {
            app.manage(commands::ClipboardState::new());
            Ok(())
        })
        .build()
}
