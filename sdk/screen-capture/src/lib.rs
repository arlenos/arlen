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
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
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
    width: i32,
    height: i32,
}

/// A capturable output for the caller to choose from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInfo {
    /// The output's index, as passed to [`capture_output`] / [`capture_region`].
    pub index: usize,
    /// The output's connector name (e.g. `eDP-1`), when the compositor sent it.
    pub name: Option<String>,
    /// Current-mode width in pixels.
    pub width: i32,
    /// Current-mode height in pixels.
    pub height: i32,
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
            state.outputs.push(OutputBinding {
                output,
                name: None,
                width: 0,
                height: 0,
            });
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
        })
        .collect())
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

/// Capture a rectangular region of output `output_index`: capture the whole output,
/// then crop to `(x, y, w, h)` in output pixels (the compositor copies the frame,
/// the crop is client-side, exactly as grim does `-g`).
pub fn capture_region(
    output_index: usize,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Result<CapturedImage> {
    capture_output(output_index)?.crop(x, y, w, h)
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
/// compositor's shm pixels to RGBA. Fails if the output is absent, the compositor
/// offers no format we can convert, or the frame copy fails.
pub fn capture_output(output_index: usize) -> Result<CapturedImage> {
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

    let source = source_manager.create_source(&output, &qh, ());
    let session = copy_manager.create_session(&source, Options::empty(), &qh, ());

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
}
