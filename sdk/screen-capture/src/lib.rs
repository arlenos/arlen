//! The Arlen screen-capture core.
//!
//! A Wayland client that drives `ext-image-copy-capture-v1` DIRECTLY against the
//! compositor (screenshot-capture-plan.md §1-2). This is deliberate: a first-party
//! tool does not go through the xdg portal (that layer mediates capture for
//! *untrusted* apps), and it does not use `wlr-screencopy` (officially deprecated,
//! and Smithay ships no server-side handler for it). The compositor exposes the ext
//! protocol server-side (smithay `ImageCopyCaptureState`), so we drive an existing
//! path rather than invent one.
//!
//! This crate is the reusable capture library shared by the first-party screenshot
//! tool and the portal Screenshot / ScreenCast backends. It is being built
//! incrementally, each slice runtime-verified under a nested compositor
//! (`dev/screenshot/shoot-compositor.sh`). Slice 1 (here) is connection + global
//! enumeration: confirm the client reaches the compositor and that it advertises
//! the two capture managers. The capture session, buffers, and PNG output land in
//! the next slices.

use anyhow::{Context, Result};
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};

/// The `ext-image-copy-capture` frame-copy manager interface. Its presence means
/// the compositor can hand us composited frames.
pub const COPY_MANAGER_INTERFACE: &str = "ext_image_copy_capture_manager_v1";
/// The `ext-image-capture-source` output-source factory interface. Its presence
/// means we can name a monitor as a capture source.
pub const OUTPUT_SOURCE_MANAGER_INTERFACE: &str = "ext_output_image_capture_source_manager_v1";
/// The `ext-image-capture-source` foreign-toplevel-source factory: a window as a
/// capture source (used by window-capture mode in a later slice).
pub const TOPLEVEL_SOURCE_MANAGER_INTERFACE: &str =
    "ext_foreign_toplevel_image_capture_source_manager_v1";

/// One advertised Wayland global: its interface name and the version the compositor
/// offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvertisedGlobal {
    /// The interface name (e.g. `ext_image_copy_capture_manager_v1`).
    pub interface: String,
    /// The version the compositor advertises.
    pub version: u32,
}

/// What the compositor offers for capture, resolved from the advertised globals.
#[derive(Debug, Clone)]
pub struct CaptureSupport {
    /// Every advertised global (interface, version).
    pub globals: Vec<AdvertisedGlobal>,
}

impl CaptureSupport {
    /// The advertised version of `interface`, if present.
    pub fn version_of(&self, interface: &str) -> Option<u32> {
        self.globals
            .iter()
            .find(|g| g.interface == interface)
            .map(|g| g.version)
    }

    /// Whether the compositor advertises the frame-copy manager (the load-bearing
    /// global: without it we cannot capture at all).
    pub fn has_copy_manager(&self) -> bool {
        self.version_of(COPY_MANAGER_INTERFACE).is_some()
    }

    /// Whether the compositor advertises the output capture-source factory (needed
    /// to capture a whole monitor).
    pub fn has_output_source_manager(&self) -> bool {
        self.version_of(OUTPUT_SOURCE_MANAGER_INTERFACE).is_some()
    }

    /// Whether the compositor advertises the foreign-toplevel capture-source factory
    /// (needed for window capture).
    pub fn has_toplevel_source_manager(&self) -> bool {
        self.version_of(TOPLEVEL_SOURCE_MANAGER_INTERFACE).is_some()
    }
}

/// The client state. Slice 1 only enumerates the registry, so it carries nothing
/// yet; capture sessions and collected outputs are added in later slices.
#[derive(Default)]
struct AppData;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for AppData {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Registry churn (globals appearing/disappearing) is not relevant to a
        // one-shot capture; `GlobalListContents` tracks the live set for us.
    }
}

/// Connect to the Wayland compositor (`$WAYLAND_DISPLAY`) and report what it offers
/// for capture, from the advertised globals. Fails if there is no compositor to
/// connect to. This is the slice-1 probe; it does not yet create a capture session.
pub fn capture_support() -> Result<CaptureSupport> {
    let conn = Connection::connect_to_env().context("connect to the Wayland compositor")?;
    let (globals, _queue) =
        registry_queue_init::<AppData>(&conn).context("initialise the Wayland registry")?;
    let mut list: Vec<AdvertisedGlobal> = globals
        .contents()
        .clone_list()
        .into_iter()
        .map(|g| AdvertisedGlobal {
            interface: g.interface,
            version: g.version,
        })
        .collect();
    list.sort_by(|a, b| a.interface.cmp(&b.interface));
    Ok(CaptureSupport { globals: list })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_resolves_the_capture_interfaces() {
        let support = CaptureSupport {
            globals: vec![
                AdvertisedGlobal { interface: COPY_MANAGER_INTERFACE.into(), version: 1 },
                AdvertisedGlobal {
                    interface: OUTPUT_SOURCE_MANAGER_INTERFACE.into(),
                    version: 1,
                },
                AdvertisedGlobal { interface: "wl_output".into(), version: 4 },
            ],
        };
        assert!(support.has_copy_manager());
        assert!(support.has_output_source_manager());
        assert!(!support.has_toplevel_source_manager());
        assert_eq!(support.version_of("wl_output"), Some(4));
        assert_eq!(support.version_of("nope"), None);
    }
}
