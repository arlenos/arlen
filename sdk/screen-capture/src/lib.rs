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

use std::os::fd::AsFd;

use anyhow::{anyhow, Context, Result};
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{event_created_child, Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::{
    self, ExtForeignToplevelHandleV1,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_list_v1::{
    self, ExtForeignToplevelListV1,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_frame_v1::{
    self, ExtImageCopyCaptureFrameV1,
};
use wayland_protocols::ext::image_capture_source::v1::client::ext_image_capture_source_v1::ExtImageCaptureSourceV1;
use wayland_protocols::ext::image_capture_source::v1::client::ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1;
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_manager_v1::{
    ExtImageCopyCaptureManagerV1, Options,
};
use wayland_protocols::ext::image_copy_capture::v1::client::ext_image_copy_capture_session_v1::{
    self, ExtImageCopyCaptureSessionV1,
};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::ZxdgOutputManagerV1;
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};

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

/// A bound output: the proxy, its human name (`name` event), and its current mode
/// pixel dimensions (`mode` event with the `current` flag).
struct OutputBinding {
    output: wl_output::WlOutput,
    name: Option<String>,
    /// Physical (capture-buffer) pixel dimensions, from the `mode` event.
    width: i32,
    height: i32,
    /// Logical position + size in the compositor's global layout, from xdg-output.
    /// The ratio physical/logical is the (possibly fractional) output scale.
    logical_x: i32,
    logical_y: i32,
    logical_width: i32,
    logical_height: i32,
    /// The xdg-output handle, kept alive so its events keep arriving.
    _xdg: Option<ZxdgOutputV1>,
}

/// A capturable output for the caller to choose from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInfo {
    /// The output's index, as passed to [`capture_output`] / [`capture_region`].
    pub index: usize,
    /// The output's connector name (e.g. `eDP-1`), when the compositor sent it.
    pub name: Option<String>,
    /// Current-mode width in physical pixels.
    pub width: i32,
    /// Current-mode height in physical pixels.
    pub height: i32,
    /// Logical position in the compositor's global layout (xdg-output).
    pub logical_x: i32,
    /// Logical y position (xdg-output).
    pub logical_y: i32,
    /// Logical width; `physical/logical` is the output scale (0 if unknown).
    pub logical_width: i32,
    /// Logical height (xdg-output).
    pub logical_height: i32,
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
    /// The frame copy completed (`ready`): the attached buffer now holds pixels.
    frame_ready: bool,
    /// The frame copy failed (`failed`), with the reason string if given.
    frame_failed: Option<String>,
    /// The toplevel windows the compositor advertised (foreign-toplevel-list).
    windows: Vec<WindowBinding>,
}

/// A bound foreign toplevel: its handle plus title/app_id once those events arrive.
struct WindowBinding {
    handle: ExtForeignToplevelHandleV1,
    title: Option<String>,
    app_id: Option<String>,
}

/// A capturable window for the caller to choose from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// The window's index, for a future window-capture call.
    pub index: usize,
    /// The window title, when the compositor sent one.
    pub title: Option<String>,
    /// The window's app id, when the compositor sent one.
    pub app_id: Option<String>,
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
        let Some(b) = state.outputs.iter_mut().find(|b| &b.output == proxy) else {
            return;
        };
        match event {
            wl_output::Event::Name { name } => b.name = Some(name),
            wl_output::Event::Mode { flags, width, height, .. } => {
                // Record only the current mode (the one being displayed).
                if let wayland_client::WEnum::Value(m) = flags {
                    if m.contains(wl_output::Mode::Current) {
                        b.width = width;
                        b.height = height;
                    }
                }
            }
            _ => {}
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

impl Dispatch<ZxdgOutputManagerV1, ()> for CaptureState {
    fn event(
        _: &mut Self,
        _: &ZxdgOutputManagerV1,
        _: <ZxdgOutputManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZxdgOutputV1, usize> for CaptureState {
    fn event(
        state: &mut Self,
        _: &ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        index: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(b) = state.outputs.get_mut(*index) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                b.logical_x = x;
                b.logical_y = y;
            }
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                b.logical_width = width;
                b.logical_height = height;
            }
            _ => {}
        }
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

impl Dispatch<ExtForeignToplevelListV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state.windows.push(WindowBinding {
                handle: toplevel,
                title: None,
                app_id: None,
            });
        }
    }

    event_created_child!(CaptureState, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        proxy: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(w) = state.windows.iter_mut().find(|w| &w.handle == proxy) else {
            return;
        };
        match event {
            ext_foreign_toplevel_handle_v1::Event::Title { title } => w.title = Some(title),
            ext_foreign_toplevel_handle_v1::Event::AppId { app_id } => w.app_id = Some(app_id),
            _ => {}
        }
    }
}

/// Enumerate the capturable windows (foreign toplevels) with their title + app id,
/// so a caller can pick one for window capture.
pub fn list_windows() -> Result<Vec<WindowInfo>> {
    let conn = Connection::connect_to_env().context("connect to the Wayland compositor")?;
    let (globals, mut queue) =
        registry_queue_init::<CaptureState>(&conn).context("initialise the Wayland registry")?;
    let qh = queue.handle();
    let mut state = CaptureState::default();
    let _list = globals
        .bind::<ExtForeignToplevelListV1, _, _>(&qh, 1..=1, ())
        .map_err(|e| anyhow!("bind ext_foreign_toplevel_list_v1: {e}"))?;
    // The compositor sends a `toplevel` per window, then each window's title/app_id;
    // two roundtrips settle the initial set.
    queue.roundtrip(&mut state).context("roundtrip for the toplevel set")?;
    queue.roundtrip(&mut state).context("roundtrip for toplevel metadata")?;
    Ok(state
        .windows
        .iter()
        .enumerate()
        .map(|(index, w)| WindowInfo {
            index,
            title: w.title.clone(),
            app_id: w.app_id.clone(),
        })
        .collect())
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
            state.outputs.push(OutputBinding {
                output,
                name: None,
                width: 0,
                height: 0,
                logical_x: 0,
                logical_y: 0,
                logical_width: 0,
                logical_height: 0,
                _xdg: None,
            });
        }
    }
    // xdg-output gives the logical position + size, and thus the (possibly
    // fractional) output scale that wl_output's integer `scale` cannot express.
    // Optional: if the compositor lacks it, logical geometry stays zero and callers
    // fall back to physical coordinates.
    if let Ok(xdg_manager) = globals.bind::<ZxdgOutputManagerV1, _, _>(qh, 1..=3, ()) {
        for (index, b) in state.outputs.iter_mut().enumerate() {
            b._xdg = Some(xdg_manager.get_xdg_output(&b.output, qh, index));
        }
    }
    Ok((source_manager, copy_manager))
}

/// Enumerate the capturable outputs (monitors) with their names and current-mode
/// pixel dimensions, so a caller can pick one by index or name.
pub fn list_outputs() -> Result<Vec<OutputInfo>> {
    let conn = Connection::connect_to_env().context("connect to the Wayland compositor")?;
    let (globals, mut queue) =
        registry_queue_init::<CaptureState>(&conn).context("initialise the Wayland registry")?;
    let qh = queue.handle();
    let mut state = CaptureState::default();
    let _ = bind_capture_globals(&globals, &qh, &mut state)?;
    queue.roundtrip(&mut state).context("roundtrip for output info")?;
    Ok(state
        .outputs
        .iter()
        .enumerate()
        .map(|(index, b)| OutputInfo {
            index,
            name: b.name.clone(),
            width: b.width,
            height: b.height,
            logical_x: b.logical_x,
            logical_y: b.logical_y,
            logical_width: b.logical_width,
            logical_height: b.logical_height,
        })
        .collect())
}

/// Map a rectangle given in the compositor's global LOGICAL coordinates (the
/// convention grim's `-g` and the desktop portal use) to physical capture-buffer
/// pixels for `output`, honouring a fractional scale. Falls back to treating the
/// input as physical if the output has no logical geometry (no xdg-output).
fn logical_to_physical_rect(
    output: &OutputInfo,
    lx: u32,
    ly: u32,
    lw: u32,
    lh: u32,
) -> (u32, u32, u32, u32) {
    if output.logical_width <= 0 || output.logical_height <= 0 {
        return (lx, ly, lw, lh);
    }
    let sx = output.width as f64 / output.logical_width as f64;
    let sy = output.height as f64 / output.logical_height as f64;
    let ox = (lx as i64 - output.logical_x as i64).max(0) as f64;
    let oy = (ly as i64 - output.logical_y as i64).max(0) as f64;
    (
        (ox * sx).round() as u32,
        (oy * sy).round() as u32,
        (lw as f64 * sx).round() as u32,
        (lh as f64 * sy).round() as u32,
    )
}

impl CapturedImage {
    /// Crop to the rectangle `(x, y, w, h)` in this image's pixel space, clamped to
    /// the image bounds (so an over-large region yields the on-screen part). Returns
    /// an error if the rectangle starts outside the image.
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Result<CapturedImage> {
        if x >= self.width || y >= self.height {
            return Err(anyhow!(
                "region origin ({x},{y}) is outside the {}x{} capture",
                self.width,
                self.height
            ));
        }
        let cw = w.min(self.width - x);
        let ch = h.min(self.height - y);
        let mut rgba = Vec::with_capacity(cw as usize * ch as usize * 4);
        let src_stride = self.width as usize * 4;
        for row in 0..ch as usize {
            let sy = y as usize + row;
            let start = sy * src_stride + x as usize * 4;
            rgba.extend_from_slice(&self.rgba[start..start + cw as usize * 4]);
        }
        Ok(CapturedImage {
            width: cw,
            height: ch,
            rgba,
        })
    }
}

/// Capture a rectangular region of output `output_index`. The region is given in
/// the compositor's global LOGICAL coordinates (the grim `-g` / portal convention);
/// it is mapped to physical capture-buffer pixels via the output's (possibly
/// fractional) scale, then the captured frame is cropped client-side.
pub fn capture_region(
    output_index: usize,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    include_cursor: bool,
) -> Result<CapturedImage> {
    let outputs = list_outputs()?;
    let output = outputs
        .get(output_index)
        .ok_or_else(|| anyhow!("output index {output_index} out of range"))?;
    let (px, py, pw, ph) = logical_to_physical_rect(output, x, y, w, h);
    capture_output(output_index, include_cursor)?.crop(px, py, pw, ph)
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

impl Dispatch<wl_shm_pool::WlShmPool, ()> for CaptureState {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_buffer::WlBuffer, ()> for CaptureState {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The only event is `release`; for a one-shot capture we read the buffer
        // once the frame is `ready` and are done with it, so releasing is moot.
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for CaptureState {
    fn event(
        state: &mut Self,
        _: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        match event {
            Event::Ready => state.frame_ready = true,
            Event::Failed { reason } => {
                state.frame_failed = Some(format!("{reason:?}"));
            }
            // transform / damage / presentation_time are metadata we do not need
            // for a still capture.
            _ => {}
        }
    }
}

/// A captured image: tightly-packed RGBA8 pixels, row-major, top-left origin.
#[derive(Debug, Clone)]
pub struct CapturedImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA8.
    pub rgba: Vec<u8>,
}

/// Where the R/G/B/A bytes sit within a 4-byte pixel of a given `wl_shm`/DRM
/// format code. Memory byte order is the little-endian of the DRM `MSB:..:LSB` word.
struct PixelLayout {
    r: usize,
    g: usize,
    b: usize,
    a: Option<usize>,
}

/// The byte layout for the 32-bit 8888 formats we can convert to RGBA, or `None`
/// for a format we do not (yet) handle. Covers the `wl_shm` short codes (0/1) and
/// the DRM fourccs for {A,X}{R,B}GB8888.
fn pixel_layout(code: u32) -> Option<PixelLayout> {
    match code {
        // ARGB8888: wl_shm 0, DRM 'AR24' -> word A:R:G:B -> mem B,G,R,A
        0 | 0x3432_5241 => Some(PixelLayout { r: 2, g: 1, b: 0, a: Some(3) }),
        // XRGB8888: wl_shm 1, DRM 'XR24' -> word X:R:G:B -> mem B,G,R,X
        1 | 0x3432_5258 => Some(PixelLayout { r: 2, g: 1, b: 0, a: None }),
        // ABGR8888: DRM 'AB24' -> word A:B:G:R -> mem R,G,B,A
        0x3432_4241 => Some(PixelLayout { r: 0, g: 1, b: 2, a: Some(3) }),
        // XBGR8888: DRM 'XB24' -> word X:B:G:R -> mem R,G,B,X
        0x3432_4258 => Some(PixelLayout { r: 0, g: 1, b: 2, a: None }),
        _ => None,
    }
}

/// An allocated shm buffer backed by a memfd, kept alive for the capture.
struct ShmBuffer {
    _file: std::fs::File,
    map: memmap2::MmapMut,
    buffer: wl_buffer::WlBuffer,
    stride: usize,
}

/// Allocate an shm buffer of `width*height` in `format` (a 4-bpp code) via a memfd,
/// and wrap it in a `wl_buffer` the compositor can copy into.
fn alloc_shm_buffer(
    shm: &wl_shm::WlShm,
    qh: &QueueHandle<CaptureState>,
    width: u32,
    height: u32,
    format_code: u32,
) -> Result<ShmBuffer> {
    let stride = width as usize * 4;
    let size = stride * height as usize;
    let fd = rustix::fs::memfd_create("arlen-screenshot", rustix::fs::MemfdFlags::CLOEXEC)
        .context("memfd_create for the capture buffer")?;
    let file = std::fs::File::from(fd);
    file.set_len(size as u64).context("size the capture buffer")?;
    let map = unsafe { memmap2::MmapMut::map_mut(&file).context("mmap the capture buffer")? };
    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let format = wl_shm::Format::try_from(format_code)
        .map_err(|_| anyhow!("unsupported shm format code {format_code}"))?;
    let buffer = pool.create_buffer(0, width as i32, height as i32, stride as i32, format, qh, ());
    // The pool can be destroyed immediately; the buffer keeps the mapping mapped.
    pool.destroy();
    Ok(ShmBuffer { _file: file, map, buffer, stride })
}

/// Capture output `output_index` to an RGBA image, driving the full
/// source -> session -> shm buffer -> frame -> copy handshake and converting the
/// compositor's shm pixels to RGBA. `include_cursor` paints the pointer onto the
/// frame (`Options::PaintCursors`). Fails if the output is absent, the compositor
/// offers no format we can convert, or the frame copy fails.
pub fn capture_output(output_index: usize, include_cursor: bool) -> Result<CapturedImage> {
    let conn = Connection::connect_to_env().context("connect to the Wayland compositor")?;
    let (globals, mut queue) =
        registry_queue_init::<CaptureState>(&conn).context("initialise the Wayland registry")?;
    let qh = queue.handle();
    let mut state = CaptureState::default();

    let (source_manager, copy_manager) = bind_capture_globals(&globals, &qh, &mut state)?;
    let shm = state
        .shm
        .clone()
        .ok_or_else(|| anyhow!("the compositor advertises no wl_shm"))?;
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

    let options = if include_cursor {
        Options::PaintCursors
    } else {
        Options::empty()
    };
    let source = source_manager.create_source(&output, &qh, ());
    let session = copy_manager.create_session(&source, options, &qh, ());

    // Wait for the buffer constraints.
    while !state.session_done && !state.session_stopped {
        queue
            .blocking_dispatch(&mut state)
            .context("dispatch capture-session events")?;
    }
    let (width, height) = state
        .buffer_size
        .ok_or_else(|| anyhow!("the capture session reported no buffer size"))?;

    // Pick the first offered format we can convert.
    let format_code = state
        .session_shm_formats
        .iter()
        .copied()
        .find(|c| pixel_layout(*c).is_some())
        .ok_or_else(|| {
            anyhow!(
                "no convertible shm format among {:?}",
                state.session_shm_formats
            )
        })?;
    let layout = pixel_layout(format_code).expect("checked above");

    let shm_buffer = alloc_shm_buffer(&shm, &qh, width, height, format_code)?;

    // Create the frame, attach the buffer, and request the copy.
    let frame = session.create_frame(&qh, ());
    frame.attach_buffer(&shm_buffer.buffer);
    frame.capture();

    // Wait for the copy to land.
    state.frame_ready = false;
    while !state.frame_ready && state.frame_failed.is_none() {
        queue
            .blocking_dispatch(&mut state)
            .context("dispatch capture-frame events")?;
    }
    if let Some(reason) = &state.frame_failed {
        return Err(anyhow!("capture frame failed: {reason}"));
    }

    // Convert the shm pixels to tightly-packed RGBA.
    let src = &shm_buffer.map[..];
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height as usize {
        let row = &src[y * shm_buffer.stride..];
        for x in 0..width as usize {
            let px = &row[x * 4..x * 4 + 4];
            rgba.push(px[layout.r]);
            rgba.push(px[layout.g]);
            rgba.push(px[layout.b]);
            rgba.push(layout.a.map(|i| px[i]).unwrap_or(255));
        }
    }

    frame.destroy();
    session.destroy();
    Ok(CapturedImage { width, height, rgba })
}

/// Write a [`CapturedImage`] to `path` as a PNG (RGBA8).
pub fn write_png(image: &CapturedImage, path: &std::path::Path) -> Result<()> {
    let file = std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().context("write PNG header")?;
    writer
        .write_image_data(&image.rgba)
        .context("write PNG pixels")?;
    Ok(())
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

    #[test]
    fn crop_extracts_the_subrect_and_clamps() {
        // A 3x2 image whose red channel encodes the pixel index (y*3 + x).
        let mut rgba = Vec::new();
        for y in 0..2u8 {
            for x in 0..3u8 {
                rgba.extend_from_slice(&[y * 3 + x, 0, 0, 255]);
            }
        }
        let img = CapturedImage { width: 3, height: 2, rgba };

        let c = img.crop(1, 0, 2, 2).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.rgba[0], 1, "top-left of the crop is pixel (1,0)");
        assert_eq!(c.rgba[4], 2, "next is pixel (2,0)");

        // An over-large region clamps to the image bounds.
        let clamped = img.crop(2, 1, 99, 99).unwrap();
        assert_eq!((clamped.width, clamped.height), (1, 1));

        // An origin outside the image is an error.
        assert!(img.crop(3, 0, 1, 1).is_err());
    }

    fn out(w: i32, h: i32, lx: i32, ly: i32, lw: i32, lh: i32) -> OutputInfo {
        OutputInfo {
            index: 0,
            name: None,
            width: w,
            height: h,
            logical_x: lx,
            logical_y: ly,
            logical_width: lw,
            logical_height: lh,
        }
    }

    #[test]
    fn logical_region_maps_to_physical_with_scale() {
        // A 2x output (2000x1000 physical, 1000x500 logical) at the origin.
        let o = out(2000, 1000, 0, 0, 1000, 500);
        assert_eq!(
            logical_to_physical_rect(&o, 100, 50, 300, 200),
            (200, 100, 600, 400)
        );

        // No logical geometry -> the input is treated as physical (passthrough).
        let none = out(1920, 1080, 0, 0, 0, 0);
        assert_eq!(logical_to_physical_rect(&none, 10, 20, 30, 40), (10, 20, 30, 40));

        // A second monitor placed to the right (logical origin 1000,0), 1x scale:
        // a global-logical x of 1100 maps to physical x 100 on that output.
        let right = out(1920, 1080, 1000, 0, 1920, 1080);
        assert_eq!(
            logical_to_physical_rect(&right, 1100, 10, 100, 50),
            (100, 10, 100, 50)
        );
    }
}
