/// Display panel backend.
///
/// `wayland_client` runs a dedicated `std::thread` that owns the
/// `wlr-output-management` Wayland connection. Commands flow into
/// the thread through an `mpsc::Sender`; live monitor state is
/// published into a `Arc<Mutex<DisplayState>>` and pushed to the
/// frontend as Tauri `displays:changed` events.
///
/// `types` defines the snapshot types the Tauri commands hand back
/// to the frontend. They are intentionally separate from the
/// canonical `OutputConfig` (used on disk) so the frontend has a
/// camelCase, JSON-friendly view that does not depend on the
/// compositor's struct shape evolving.

pub mod profiles;
pub mod types;
pub mod wayland_client;

pub use types::{Monitor, MonitorConfig, MonitorMode, Position, Transform};
pub use wayland_client::{spawn, WaylandCommand, WaylandHandle};
