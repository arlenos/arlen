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

use anyhow::{anyhow, Context, Result};
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{wl_output, wl_registry, wl_shm};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    ExtImageCopyCaptureManagerV1, Options,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self, ExtImageCopyCaptureSessionV1,
};

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

/// The buffer constraints a capture session advertises for a source: the pixel
/// dimensions the compositor will copy, and the shm formats it offers. Collected
/// from the session's `buffer_size` / `shm_format` events, complete at `done`.
#[derive(Debug, Clone, Default)]
pub struct SessionConstraints {
    /// The capture buffer width in pixels.
    pub width: u32,
    /// The capture buffer height in pixels.
    pub height: u32,
    /// The shm pixel formats (`wl_shm.format` codes) the compositor will copy into,
    /// in the order it offered them.
    pub shm_formats: Vec<u32>,
}

/// A bound output: the proxy plus its human name once the `name` event arrives.
struct OutputBinding {
    output: wl_output::WlOutput,
    name: Option<String>,
}

/// Client state for a capture flow. Hand-rolled (no sctk) so the capture protocol
/// stays explicit end to end; the collected session constraints live here.
#[derive(Default)]
struct CaptureState {
    outputs: Vec<OutputBinding>,
    shm: Option<wl_shm::WlShm>,
    /// Buffer size the session reported (`buffer_size`).
    buffer_size: Option<(u32, u32)>,
    /// Shm formats the session offered (`shm_format`), in order.
    session_shm_formats: Vec<u32>,
    /// The session finished reporting its constraints (`done`).
    session_done: bool,
    /// The session stopped (`stopped`) - the source went away.
    session_stopped: bool,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for CaptureState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for CaptureState {
    fn event(
        state: &mut Self,
        proxy: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event {
            if let Some(b) = state.outputs.iter_mut().find(|b| &b.output == proxy) {
                b.name = Some(name);
            }
        }
    }
}

impl Dispatch<wl_shm::WlShm, ()> for CaptureState {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The global `wl_shm.format` list is not needed; the session reports the
        // formats it will actually copy into.
    }
}

impl Dispatch<ExtOutputImageCaptureSourceManagerV1, ()> for CaptureState {
    fn event(
        _: &mut Self,
        _: &ExtOutputImageCaptureSourceManagerV1,
        _: <ExtOutputImageCaptureSourceManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtImageCaptureSourceV1, ()> for CaptureState {
    fn event(
        _: &mut Self,
        _: &ExtImageCaptureSourceV1,
        _: <ExtImageCaptureSourceV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for CaptureState {
    fn event(
        _: &mut Self,
        _: &ExtImageCopyCaptureManagerV1,
        _: <ExtImageCopyCaptureManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        match event {
            Event::BufferSize { width, height } => state.buffer_size = Some((width, height)),
            Event::ShmFormat { format } => {
                // `format` is a `WEnum<wl_shm::Format>`; keep the raw code so the
                // buffer allocator can match it directly.
                state.session_shm_formats.push(u32::from(format));
            }
            Event::Done => state.session_done = true,
            Event::Stopped => state.session_stopped = true,
            _ => {}
        }
    }
}

/// Bind the capture managers, shm, and every output from an initialised registry.
fn bind_capture_globals(
    globals: &GlobalList,
    qh: &QueueHandle<CaptureState>,
    state: &mut CaptureState,
) -> Result<(ExtOutputImageCaptureSourceManagerV1, ExtImageCopyCaptureManagerV1)> {
    state.shm = globals.bind::<wl_shm::WlShm, _, _>(qh, 1..=1, ()).ok();
    let source_manager = globals
        .bind::<ExtOutputImageCaptureSourceManagerV1, _, _>(qh, 1..=1, ())
        .map_err(|e| anyhow!("bind ext_output_image_capture_source_manager_v1: {e}"))?;
    let copy_manager = globals
        .bind::<ExtImageCopyCaptureManagerV1, _, _>(qh, 1..=1, ())
        .map_err(|e| anyhow!("bind ext_image_copy_capture_manager_v1: {e}"))?;
    for g in globals.contents().clone_list() {
        if g.interface == wl_output::WlOutput::interface().name {
            let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
                g.name,
                g.version.min(4),
                qh,
                (),
            );
            state.outputs.push(OutputBinding { output, name: None });
        }
    }
    Ok((source_manager, copy_manager))
}

/// Create a capture session for output `output_index` and return the buffer
/// constraints the compositor reports (size + shm formats). This exercises the full
/// source -> session -> constraints handshake without yet allocating a buffer or
/// copying a frame (those are the next slice). Fails if there is no such output or
/// the compositor lacks the capture managers.
pub fn probe_session(output_index: usize) -> Result<SessionConstraints> {
    let conn = Connection::connect_to_env().context("connect to the Wayland compositor")?;
    let (globals, mut queue) =
        registry_queue_init::<CaptureState>(&conn).context("initialise the Wayland registry")?;
    let qh = queue.handle();
    let mut state = CaptureState::default();

    let (source_manager, copy_manager) = bind_capture_globals(&globals, &qh, &mut state)?;
    // Learn the output names + let the registry settle.
    queue.roundtrip(&mut state).context("initial roundtrip")?;

    let output = state
        .outputs
        .get(output_index)
        .ok_or_else(|| {
            anyhow!(
                "output index {output_index} out of range ({} outputs)",
                state.outputs.len()
            )
        })?
        .output
        .clone();

    let source = source_manager.create_source(&output, &qh, ());
    let _session = copy_manager.create_session(&source, Options::empty(), &qh, ());

    // Dispatch until the session finishes reporting constraints (`done`) or stops.
    while !state.session_done && !state.session_stopped {
        queue
            .blocking_dispatch(&mut state)
            .context("dispatch capture-session events")?;
    }
    let (width, height) = state
        .buffer_size
        .ok_or_else(|| anyhow!("the capture session reported no buffer size"))?;
    Ok(SessionConstraints {
        width,
        height,
        shm_formats: state.session_shm_formats.clone(),
    })
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
