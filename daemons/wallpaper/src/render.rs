//! WP-R1 renderer: a `wlr-layer-shell` background client.
//!
//! For every output the compositor exposes, this creates a `Background`-layer
//! surface covering the whole output, composes the configured wallpaper for
//! that output ([`crate::decode::frame_for_output`] does the decode + Fill/Zoom
//! scale - all the pixel work), copies it into a `wl_shm` buffer, and commits.
//! The pixel pipeline is pure + tested; this module is only the Wayland
//! plumbing (registry, outputs, layer shell, shm) via smithay-client-toolkit.
//!
//! Static images only: a `Video`/`Shader` manifest yields no frame here (the
//! sandboxed live renderer, WP-R2, owns those) so the surface shows the flat
//! fallback rather than nothing.

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    registry_handlers,
};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::{wl_output, wl_shm, wl_surface};
use wayland_client::{Connection, EventQueue, QueueHandle};

use crate::decode::frame_for_output;
use crate::manifest::WallpaperManifest;
use crate::schedule::TimeContext;

/// Opaque black: the letterbox margin + the fallback fill (RGBA).
const FALLBACK: [u8; 4] = [0, 0, 0, 255];

/// One output's background surface + the geometry the compositor gave it.
struct Background {
    layer: LayerSurface,
    connector: String,
    width: u32,
    height: u32,
}

/// The wallpaper client state, dispatched by the smithay-client-toolkit
/// handlers below.
pub struct Wallpaper {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    compositor: CompositorState,
    layer_shell: LayerShell,
    manifest: WallpaperManifest,
    time: TimeContext,
    backgrounds: Vec<Background>,
}

impl Wallpaper {
    /// Bind the globals and build the client. Returns the state + its event
    /// queue; the caller drives `blocking_dispatch` in a loop.
    pub fn new(
        conn: &Connection,
        manifest: WallpaperManifest,
        time: TimeContext,
    ) -> Result<(Self, EventQueue<Self>), Box<dyn std::error::Error>> {
        let (globals, queue) = registry_queue_init::<Self>(conn)?;
        let qh = queue.handle();
        let shm = Shm::bind(&globals, &qh)?;
        let pool = SlotPool::new(256 * 256 * 4, &shm)?;
        let state = Self {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            shm,
            pool,
            compositor: CompositorState::bind(&globals, &qh)?,
            layer_shell: LayerShell::bind(&globals, &qh)?,
            manifest,
            time,
            backgrounds: Vec::new(),
        };
        Ok((state, queue))
    }

    /// Create a full-output background surface for `output`.
    fn add_output(&mut self, qh: &QueueHandle<Self>, output: &wl_output::WlOutput) {
        let connector = self
            .output_state
            .info(output)
            .and_then(|i| i.name)
            .unwrap_or_default();
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Background,
            Some("arlen-wallpaper"),
            Some(output),
        );
        layer.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer.set_exclusive_zone(-1);
        layer.set_size(0, 0);
        layer.commit();
        self.backgrounds.push(Background {
            layer,
            connector,
            width: 0,
            height: 0,
        });
    }

    /// The index of the background whose layer owns `surface`, if any.
    fn index_of(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.backgrounds
            .iter()
            .position(|b| b.layer.wl_surface() == surface)
    }

    /// Compose the wallpaper for background `index` into a fresh shm buffer and
    /// commit it. No-op until the compositor has configured a real size.
    fn draw(&mut self, index: usize) {
        let (w, h, connector) = {
            let bg = &self.backgrounds[index];
            (bg.width, bg.height, bg.connector.clone())
        };
        if w == 0 || h == 0 {
            return;
        }
        let stride = w as i32 * 4;
        let (buffer, canvas) =
            match self
                .pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
            {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("wallpaper shm buffer: {e}");
                    return;
                }
            };

        match frame_for_output(&self.manifest, &connector, &self.time, w, h, FALLBACK) {
            // The composed frame is RGBA; wl_shm Argb8888 is little-endian, i.e.
            // BGRA byte order. Reorder as we copy.
            Some(rgba) if rgba.len() == canvas.len() => {
                for (dst, src) in canvas.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
                    dst[0] = src[2];
                    dst[1] = src[1];
                    dst[2] = src[0];
                    dst[3] = src[3];
                }
            }
            // A Video/Shader wallpaper (WP-R2's job), a decode failure, or a size
            // mismatch: paint the flat fallback rather than leave uninitialised.
            _ => {
                let fill = [FALLBACK[2], FALLBACK[1], FALLBACK[0], FALLBACK[3]];
                for px in canvas.chunks_exact_mut(4) {
                    px.copy_from_slice(&fill);
                }
            }
        }

        let bg = &self.backgrounds[index];
        let surface = bg.layer.wl_surface();
        surface.attach(Some(buffer.wl_buffer()), 0, 0);
        surface.damage_buffer(0, 0, w as i32, h as i32);
        bg.layer.commit();
    }
}

impl CompositorHandler for Wallpaper {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {}
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for Wallpaper {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.add_output(qh, &output);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let name = self.output_state.info(&output).and_then(|i| i.name);
        if let Some(name) = name {
            self.backgrounds.retain(|b| b.connector != name);
        }
    }
}

impl LayerShellHandler for Wallpaper {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, layer: &LayerSurface) {
        self.backgrounds
            .retain(|b| b.layer.wl_surface() != layer.wl_surface());
    }
    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let Some(index) = self.index_of(layer.wl_surface()) else {
            return;
        };
        let (w, h) = configure.new_size;
        if w > 0 && h > 0 {
            self.backgrounds[index].width = w;
            self.backgrounds[index].height = h;
        }
        self.draw(index);
    }
}

impl ShmHandler for Wallpaper {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for Wallpaper {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_compositor!(Wallpaper);
delegate_output!(Wallpaper);
delegate_layer!(Wallpaper);
delegate_shm!(Wallpaper);
delegate_registry!(Wallpaper);
