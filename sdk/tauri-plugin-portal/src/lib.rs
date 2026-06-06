//! Tauri plugin wrapping `org.freedesktop.portal.Desktop` for
//! first-party Lunaris apps.
//!
//! The frontend daemon (`xdg-desktop-portal`) routes calls to
//! whichever backend is registered for the current desktop. On
//! Lunaris that is `xdg-desktop-portal-lunaris`; on other sessions
//! the plugin falls through gracefully because we never call the
//! Lunaris backend directly — only the standard frontend.
//!
//! # Public API
//!
//! See [`pick_file`], [`pick_directory`], [`save_file`],
//! [`save_files`], [`open_uri`]. The TypeScript counterparts in
//! `index.ts` mirror these one-to-one.
//!
//! # Usage (Rust)
//!
//! ```rust,ignore
//! fn main() {
//!     tauri::Builder::default()
//!         .plugin(tauri_plugin_lunaris_portal::init())
//!         .run(tauri::generate_context!())
//!         .expect("error running app");
//! }
//! ```
//!
//! For consumers that need a Rust-level (not Tauri-command) entry
//! into the plugin — typically `app-settings` reusing the picker
//! from inside its own commands — see the public `api` module.

pub mod api;
mod commands;
mod portal_proxy;
mod request_helper;
mod types;

pub use types::{
    FileFilter, FilterPattern, OpenUriOptions, PickFileOptions, PickerError,
    PickerResult, SaveFileOptions, SaveFilesOptions,
};

use tauri::{
    plugin::{Builder, TauriPlugin},
    Runtime,
};

/// Initialise the Lunaris portal plugin. Registers all five Tauri
/// commands (`pick_file`, `pick_directory`, `save_file`,
/// `save_files`, `open_uri`).
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("lunaris-portal")
        .invoke_handler(tauri::generate_handler![
            commands::pick_file,
            commands::pick_directory,
            commands::save_file,
            commands::save_files,
            commands::open_uri,
        ])
        .build()
}
