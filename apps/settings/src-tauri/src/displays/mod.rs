//! Display panel backend.
//!
//! `wayland_client` runs a dedicated `std::thread` that owns the
//! `wlr-output-management` Wayland connection. Commands flow into
//! the thread through an `mpsc::Sender`; live monitor state is
//! published into a `Arc<Mutex<DisplayState>>` and pushed to the
//! frontend as Tauri `displays:changed` events.
//!
//! `types` defines the snapshot types the Tauri commands hand back
//! to the frontend. They are intentionally separate from the
//! canonical `OutputConfig` (used on disk) so the frontend has a
//! camelCase, JSON-friendly view that does not depend on the
//! compositor's struct shape evolving.

pub mod profiles;
pub mod types;
pub mod wayland_client;

// No re-exports here. These two lines existed and reached nobody: every caller
// names the module it wants - `displays::wayland_client::spawn` in `lib.rs`,
// `wayland_client::{WaylandCommand, WaylandHandle}` in the commands - so the
// shortcut was a second spelling for the same items with no user. Adding one
// back is fine; adding one back that nothing uses is what this was.
