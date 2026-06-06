/// `wlr-output-management-unstable-v1` client.
///
/// Owns one `wayland_client::Connection` on a dedicated
/// `std::thread`. Outgoing requests come in via an `mpsc::Sender`;
/// each tick of the dispatch loop folds head/mode events into a
/// `Snapshot` of the live monitor topology and pushes it to the
/// frontend through a Tauri event.
///
/// We talk wlr-output-management (not the cosmic extension) because:
///   * The compositor implements both server sides.
///   * wlr is the cross-compositor standard, so the Settings app
///     keeps working under labwc / sway / wayfire if a user runs
///     it outside the Lunaris compositor.
///
/// The Wayland event model is asynchronous: heads, modes, and
/// per-head fields each arrive as separate events that batch on a
/// `manager.done` boundary. We accumulate them into `staging_*`
/// hash maps and only publish a snapshot when `manager.done`
/// fires, mirroring the protocol's transactional semantics.

use std::{
    collections::HashMap,
    os::fd::AsRawFd,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use wayland_client::{
    event_created_child,
    globals::{registry_queue_init, GlobalListContents},
    protocol::wl_registry,
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::ZwlrOutputConfigurationHeadV1,
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, AdaptiveSyncState, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

use super::types::{EnabledKind, Monitor, MonitorConfig, MonitorMode, Position, Transform, VrrState};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum WaylandCommand {
    /// Apply a new configuration. The `Vec` covers every monitor
    /// the frontend wants on. Outputs not mentioned are kept at
    /// their current state. The reply (`Succeeded`, `Failed`,
    /// `Cancelled`) is emitted as a `displays:apply-result`
    /// Tauri event tagged with `request_id`.
    Apply {
        request_id: String,
        config: Vec<MonitorConfig>,
    },
}

/// Handle returned by `spawn`. Cloning the handle lets callers
/// (Tauri commands) snapshot live state without going through the
/// channel.
#[derive(Clone)]
pub struct WaylandHandle {
    pub state: Arc<Mutex<DisplayState>>,
    pub sender: Sender<WaylandCommand>,
}

#[derive(Debug, Default, Clone, Serialize)]
pub struct DisplayState {
    /// Latest published snapshot (key: connector). Empty until the
    /// first `manager.done` arrives.
    pub monitors: Vec<Monitor>,
    /// Wlr-output-manager protocol serial (last `done`'s serial).
    /// Required when sending `apply()` so the compositor knows
    /// which view of the world we are configuring.
    pub serial: u32,
}

/// Spawn the wayland thread. Returns a handle plus an error if
/// the initial connect fails. Callers should treat connect-failure
/// as "no display panel available" and degrade gracefully.
///
/// The wayland connection picks up `WAYLAND_DISPLAY` by default.
/// During development we test the panel against a nested Lunaris
/// compositor while Settings itself runs on the host compositor;
/// in that case `LUNARIS_DISPLAY_WAYLAND=wayland-N` overrides only
/// the output-management connection, leaving Tauri's webview on
/// the host's regular `WAYLAND_DISPLAY`.
pub fn spawn(app: AppHandle) -> Result<WaylandHandle, ConnectError> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<WaylandCommand>();
    let state = Arc::new(Mutex::new(DisplayState::default()));
    let state_for_thread = Arc::clone(&state);

    // Block on the connect synchronously so the caller sees the
    // failure. The dispatch loop runs on its own thread once we
    // have a working connection.
    let conn = match std::env::var("LUNARIS_DISPLAY_WAYLAND") {
        Ok(name) if !name.is_empty() => {
            let runtime = std::env::var("XDG_RUNTIME_DIR")
                .map_err(|_| ConnectError::OverrideEnv("XDG_RUNTIME_DIR not set".into()))?;
            let path = std::path::PathBuf::from(runtime).join(&name);
            log::info!("displays: connecting via override path {}", path.display());
            let stream = std::os::unix::net::UnixStream::connect(&path).map_err(|e| {
                ConnectError::OverrideEnv(format!(
                    "connect {}: {e}",
                    path.display()
                ))
            })?;
            Connection::from_socket(stream)
                .map_err(|e| ConnectError::OverrideEnv(format!("from_socket: {e}")))?
        }
        _ => Connection::connect_to_env().map_err(ConnectError::Connect)?,
    };
    let (globals, queue) =
        registry_queue_init::<DispatchState>(&conn).map_err(ConnectError::Globals)?;

    // Bind the manager. Wlr-output-management goes through several
    // versions; v4 added head.adaptive_sync, which we depend on.
    let qh = queue.handle();
    log::info!(
        "displays: wayland connected, advertised globals: {}",
        globals
            .contents()
            .clone_list()
            .iter()
            .map(|g| g.interface.clone())
            .collect::<Vec<_>>()
            .join(", "),
    );
    let manager: ZwlrOutputManagerV1 = globals
        .bind(&qh, 1..=4, ())
        .map_err(|e| ConnectError::ManagerBind(e.to_string()))?;
    log::info!("displays: zwlr_output_manager_v1 bound");

    // Move the connection + queue + manager onto the dispatch thread.
    thread::Builder::new()
        .name("lunaris-settings-wayland".into())
        .spawn(move || {
            run_loop(app, conn, queue, manager, state_for_thread, cmd_rx);
        })
        .map_err(ConnectError::Thread)?;

    Ok(WaylandHandle {
        state,
        sender: cmd_tx,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("wayland connect failed: {0}")]
    Connect(#[from] wayland_client::ConnectError),
    #[error("LUNARIS_DISPLAY_WAYLAND override failed: {0}")]
    OverrideEnv(String),
    #[error("wayland globals enumeration failed: {0}")]
    Globals(wayland_client::globals::GlobalError),
    #[error("zwlr_output_manager_v1 not advertised by compositor: {0}")]
    ManagerBind(String),
    #[error("wayland thread spawn: {0}")]
    Thread(std::io::Error),
}

// ---------------------------------------------------------------------------
// Dispatch loop + state accumulator
// ---------------------------------------------------------------------------

fn run_loop(
    app: AppHandle,
    conn: Connection,
    mut queue: EventQueue<DispatchState>,
    manager: ZwlrOutputManagerV1,
    state: Arc<Mutex<DisplayState>>,
    cmd_rx: Receiver<WaylandCommand>,
) {
    let mut dispatch = DispatchState::new(app.clone(), state, manager);
    // Initial round-trip so the registry advertises what we expect
    // and the manager starts emitting its `head` events.
    if let Err(err) = queue.roundtrip(&mut dispatch) {
        log::error!("displays: initial roundtrip failed: {err}");
        return;
    }
    log::info!(
        "displays: initial roundtrip complete, {} head(s) staged",
        dispatch.staging_heads.len(),
    );

    // Main loop. The naïve "dispatch_pending + sleep + prepare_read"
    // pattern from the first cut never actually woke up: prepare_read
    // returns None while the internal queue still has events, and
    // read() does not block, so nothing was ever pulled off the
    // socket past the first roundtrip.
    //
    // Correct pattern:
    //   1. Drain Tauri-side commands (apply requests).
    //   2. flush() outgoing requests.
    //   3. dispatch_pending() to consume anything already buffered.
    //   4. prepare_read() to enter "read mode"; on the underlying
    //      FD do a poll(2) with a 100 ms timeout so the channel
    //      poll above runs at ~10 Hz even when the compositor is
    //      idle. When poll signals readability, call read() to
    //      pull events into the queue. dispatch_pending then runs
    //      next iteration.
    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            handle_command(&queue, &mut dispatch, cmd);
        }

        if let Err(err) = queue.flush() {
            log::error!("displays: flush error: {err}");
            return;
        }
        if let Err(err) = queue.dispatch_pending(&mut dispatch) {
            log::error!("displays: dispatch error: {err}");
            return;
        }

        let Some(read_guard) = queue.prepare_read() else {
            // Some events still queued — loop and dispatch them.
            continue;
        };
        let raw_fd = conn.backend().poll_fd().as_raw_fd();
        // poll(fd, POLLIN, timeout_ms). 100 ms keeps the command
        // channel responsive without burning CPU on idle waits.
        let mut pollfd = libc::pollfd {
            fd: raw_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let r = unsafe { libc::poll(&mut pollfd, 1, 100) };
        if r > 0 && (pollfd.revents & libc::POLLIN) != 0 {
            if let Err(err) = read_guard.read() {
                log::error!("displays: read error: {err}");
                return;
            }
        } else {
            // poll timed out or signalled HUP/ERR. Drop the read
            // guard so the next iteration can re-prepare.
            drop(read_guard);
        }
    }
}

fn handle_command(
    queue: &EventQueue<DispatchState>,
    dispatch: &mut DispatchState,
    cmd: WaylandCommand,
) {
    match cmd {
        WaylandCommand::Apply { request_id, config } => {
            let qh = queue.handle();
            let serial = {
                let s = dispatch.published_state.lock().unwrap();
                s.serial
            };
            let configuration: ZwlrOutputConfigurationV1 = dispatch
                .manager
                .create_configuration(serial, &qh, ApplyTag { request_id: request_id.clone() });
            for cfg in &config {
                let Some(head) = dispatch.head_by_connector(&cfg.connector) else {
                    log::warn!("displays: apply skipped {} (no live head)", cfg.connector);
                    continue;
                };
                match &cfg.enabled {
                    EnabledKind::Disabled => {
                        configuration.disable_head(&head);
                    }
                    EnabledKind::Mirror(target) => {
                        // wlr-output-management has no first-class
                        // mirror concept. The convention is: set the
                        // mirroring output's position to the target's
                        // and pick a mode with matching dimensions.
                        // The compositor's cosmic side then groups
                        // both heads into a mirror set.
                        //
                        // Each ZwlrOutputModeV1 belongs to a specific
                        // head, so we cannot reuse the target's mode
                        // proxy on the source — we must find the
                        // source's own mode that matches the target
                        // by (width, height, refresh).
                        let head_cfg = configuration.enable_head(&head, &qh, ());
                        if !apply_mirror_to_head(&head_cfg, &cfg.connector, target, dispatch) {
                            log::warn!(
                                "displays: mirror {} -> {} failed (target missing or no matching mode); leaving source independent",
                                cfg.connector,
                                target,
                            );
                            apply_to_head(&head_cfg, cfg, dispatch, &qh);
                        }
                    }
                    EnabledKind::Active => {
                        let head_cfg = configuration.enable_head(&head, &qh, ());
                        apply_to_head(&head_cfg, cfg, dispatch, &qh);
                    }
                }
            }
            // Track the request so the configuration's reply event
            // can be routed back to the right Tauri caller.
            dispatch
                .pending_applies
                .insert(configuration.id().protocol_id(), request_id);
            configuration.apply();
            // Flush so the request reaches the compositor before
            // we wait for the reply.
            let _ = queue.flush();
        }
    }
}

fn apply_to_head(
    head_cfg: &ZwlrOutputConfigurationHeadV1,
    cfg: &MonitorConfig,
    dispatch: &DispatchState,
    qh: &QueueHandle<DispatchState>,
) {
    let _ = qh; // future use: per-head logging proxy
    head_cfg.set_position(cfg.position.x, cfg.position.y);
    head_cfg.set_transform(transform_to_wenum(cfg.transform));
    // wayland-client encodes the f64 as wl_fixed (24.8) for us.
    head_cfg.set_scale(cfg.scale);
    if let Some(idx) = cfg.mode_index {
        if let Some(mode_handle) = dispatch.mode_handle(&cfg.connector, idx) {
            head_cfg.set_mode(&mode_handle);
        }
    }
    if let Some(adaptive) = match cfg.vrr {
        VrrState::Enabled => Some(true),
        VrrState::Disabled => Some(false),
        VrrState::Force => Some(true),
    } {
        // Best-effort: not all manager versions support
        // set_adaptive_sync; the call is a no-op on older ones.
        head_cfg.set_adaptive_sync(if adaptive {
            zwlr_output_head_v1::AdaptiveSyncState::Enabled
        } else {
            zwlr_output_head_v1::AdaptiveSyncState::Disabled
        });
    }
}

/// Configure `head_cfg` to mirror `target` from the `source`'s
/// perspective. Returns `false` if the target connector is unknown
/// or the source has no mode with matching dimensions — the caller
/// is expected to fall back to a plain enable in that case.
fn apply_mirror_to_head(
    head_cfg: &ZwlrOutputConfigurationHeadV1,
    source: &str,
    target: &str,
    dispatch: &DispatchState,
) -> bool {
    let state = match dispatch.published_state.lock() {
        Ok(s) => s,
        Err(_) => return false,
    };

    let target_mon = match state.monitors.iter().find(|m| m.connector == target) {
        Some(m) => m,
        None => return false,
    };
    let source_mon = match state.monitors.iter().find(|m| m.connector == source) {
        Some(m) => m,
        None => return false,
    };

    head_cfg.set_position(target_mon.position.x, target_mon.position.y);
    head_cfg.set_transform(transform_to_wenum(target_mon.transform));
    head_cfg.set_scale(target_mon.scale);

    let Some(target_mode_idx) = target_mon.current_mode else {
        return false;
    };
    let Some(target_mode) = target_mon.modes.get(target_mode_idx) else {
        return false;
    };

    let Some(idx) = match_mirror_mode(&source_mon.modes, target_mode) else {
        return false;
    };
    let Some(mode_handle) = dispatch.mode_handle(source, idx) else {
        return false;
    };
    head_cfg.set_mode(&mode_handle);
    true
}

/// Pick the source-output mode that best mirrors `target_mode`.
///
/// Dimension match is a hard requirement — the compositor only
/// groups heads as a mirror set when both sides report identical
/// `(width, height)`. Among matches, the candidate with the
/// closest refresh rate wins; this avoids picking a 24 Hz cinema
/// mode when the target runs at 60 Hz just because both share a
/// resolution. Returns `None` when no candidate satisfies the
/// dimension constraint.
fn match_mirror_mode(source_modes: &[MonitorMode], target_mode: &MonitorMode) -> Option<usize> {
    source_modes
        .iter()
        .enumerate()
        .filter(|(_, m)| m.width == target_mode.width && m.height == target_mode.height)
        .min_by_key(|(_, m)| (target_mode.refresh_mhz as i64 - m.refresh_mhz as i64).abs())
        .map(|(idx, _)| idx)
}

fn transform_to_wenum(t: Transform) -> wayland_client::protocol::wl_output::Transform {
    use wayland_client::protocol::wl_output::Transform as W;
    match t {
        Transform::Normal => W::Normal,
        Transform::Rotate90 => W::_90,
        Transform::Rotate180 => W::_180,
        Transform::Rotate270 => W::_270,
        Transform::Flipped => W::Flipped,
        Transform::Flipped90 => W::Flipped90,
        Transform::Flipped180 => W::Flipped180,
        Transform::Flipped270 => W::Flipped270,
    }
}

// ---------------------------------------------------------------------------
// Dispatch state — accumulator for head/mode events
// ---------------------------------------------------------------------------

/// User-data stored on each `ZwlrOutputConfigurationV1` so the reply
/// event can be matched back to the caller's request ID.
#[derive(Debug, Clone)]
struct ApplyTag {
    request_id: String,
}

struct StagingHead {
    name: String,
    description: String,
    make: String,
    model: String,
    serial: String,
    physical_size_mm: (i32, i32),
    modes: Vec<ZwlrOutputModeV1>,
    current_mode: Option<ZwlrOutputModeV1>,
    enabled: bool,
    position: Position,
    scale: f64,
    transform: Transform,
    adaptive_sync: bool,
}

impl Default for StagingHead {
    fn default() -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            make: String::new(),
            model: String::new(),
            serial: String::new(),
            physical_size_mm: (0, 0),
            modes: Vec::new(),
            current_mode: None,
            enabled: false,
            position: Position::default(),
            scale: 1.0,
            transform: Transform::Normal,
            adaptive_sync: false,
        }
    }
}

struct StagingMode {
    width: i32,
    height: i32,
    refresh_mhz: u32,
    preferred: bool,
}

impl Default for StagingMode {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            refresh_mhz: 0,
            preferred: false,
        }
    }
}

pub struct DispatchState {
    app: AppHandle,
    published_state: Arc<Mutex<DisplayState>>,
    manager: ZwlrOutputManagerV1,

    /// Heads still being received in the current transaction. Keyed
    /// by the head proxy's protocol id.
    staging_heads: HashMap<u32, StagingHead>,
    /// Modes still being received, keyed by the mode proxy's
    /// protocol id. We resolve the owning head via `mode_to_head`.
    staging_modes: HashMap<u32, StagingMode>,
    mode_to_head: HashMap<u32, u32>,
    /// Live head proxies, keyed by connector. Reused for `apply`.
    head_handles: HashMap<String, ZwlrOutputHeadV1>,
    /// Live mode proxies (head connector → mode index → handle).
    mode_handles: HashMap<String, Vec<ZwlrOutputModeV1>>,

    /// Apply-request reply demux. Keyed by configuration proxy id.
    pending_applies: HashMap<u32, String>,
}

impl DispatchState {
    fn new(
        app: AppHandle,
        published_state: Arc<Mutex<DisplayState>>,
        manager: ZwlrOutputManagerV1,
    ) -> Self {
        Self {
            app,
            published_state,
            manager,
            staging_heads: HashMap::new(),
            staging_modes: HashMap::new(),
            mode_to_head: HashMap::new(),
            head_handles: HashMap::new(),
            mode_handles: HashMap::new(),
            pending_applies: HashMap::new(),
        }
    }

    fn head_by_connector(&self, connector: &str) -> Option<ZwlrOutputHeadV1> {
        self.head_handles.get(connector).cloned()
    }

    fn mode_handle(&self, connector: &str, idx: usize) -> Option<ZwlrOutputModeV1> {
        self.mode_handles
            .get(connector)
            .and_then(|m| m.get(idx).cloned())
    }

    /// Promote staging into the published `DisplayState` snapshot
    /// and emit a Tauri event. Called from `manager.done` with the
    /// transaction's serial; that serial must be echoed verbatim by
    /// the next `apply()` call or the compositor cancels it.
    fn flush_to_snapshot(&mut self, serial: u32) {
        log::info!(
            "displays: flush_to_snapshot serial={} — {} staged head(s), {} mode(s)",
            serial,
            self.staging_heads.len(),
            self.staging_modes.len(),
        );
        let mut monitors: Vec<Monitor> = Vec::with_capacity(self.staging_heads.len());

        // Build a stable per-connector mode handle list so apply
        // requests have an index → handle mapping.
        let mut new_mode_handles: HashMap<String, Vec<ZwlrOutputModeV1>> = HashMap::new();

        for (head_id, head) in self.staging_heads.iter() {
            let connector = head.name.clone();

            // Collect this head's modes, sorted highest-resolution
            // first then highest-refresh first. Stable order helps
            // the side panel render dropdowns deterministically.
            let mut mode_pairs: Vec<(ZwlrOutputModeV1, MonitorMode)> = head
                .modes
                .iter()
                .filter_map(|m_handle| {
                    let s = self.staging_modes.get(&m_handle.id().protocol_id())?;
                    Some((
                        m_handle.clone(),
                        MonitorMode {
                            width: s.width,
                            height: s.height,
                            refresh_mhz: s.refresh_mhz,
                            preferred: s.preferred,
                        },
                    ))
                })
                .collect();
            mode_pairs.sort_by(|a, b| {
                b.1.width
                    .cmp(&a.1.width)
                    .then(b.1.height.cmp(&a.1.height))
                    .then(b.1.refresh_mhz.cmp(&a.1.refresh_mhz))
            });
            let modes: Vec<MonitorMode> = mode_pairs.iter().map(|(_, m)| m.clone()).collect();
            new_mode_handles.insert(
                connector.clone(),
                mode_pairs.iter().map(|(h, _)| h.clone()).collect(),
            );

            let current_mode = head.current_mode.as_ref().and_then(|cur| {
                mode_pairs
                    .iter()
                    .position(|(h, _)| h.id() == cur.id())
            });
            let preferred_mode = mode_pairs.iter().position(|(_, m)| m.preferred);

            monitors.push(Monitor {
                connector: connector.clone(),
                make: head.make.clone(),
                model: head.model.clone(),
                serial: head.serial.clone(),
                physical_size_mm: head.physical_size_mm,
                modes,
                current_mode,
                preferred_mode,
                position: head.position,
                scale: head.scale,
                transform: head.transform,
                enabled: head.enabled,
                mirroring: None, // wlr-output-management does not surface mirror groups
                vrr: VrrState::from_wlr_bool(head.adaptive_sync),
                primary: false, // wlr-output-management has no primary; cosmic extension does
                max_bpc: 0,
            });
            let _ = head_id;
        }

        monitors.sort_by(|a, b| a.connector.cmp(&b.connector));

        self.mode_handles = new_mode_handles;

        {
            let mut s = self.published_state.lock().unwrap();
            s.monitors = monitors.clone();
            s.serial = serial;
        }

        // Tell the frontend. Failure to emit (no listener yet, etc.)
        // is non-fatal — the next read of `WaylandHandle::state`
        // still sees the fresh snapshot.
        let _ = self.app.emit("displays:changed", &monitors);
    }
}

// ---------------------------------------------------------------------------
// Dispatch impls
// ---------------------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for DispatchState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: <wl_registry::WlRegistry as Proxy>::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // Globals already enumerated in `registry_queue_init`. We do
        // not subscribe to add/remove here because output add/remove
        // flows through the manager's head events.
    }
}

impl Dispatch<ZwlrOutputManagerV1, ()> for DispatchState {
    fn event(
        state: &mut Self,
        _proxy: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                let id = head.id().protocol_id();
                state.staging_heads.entry(id).or_default();
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                // The wlr-output-management protocol is transactional:
                // every `apply` request must echo back the serial of
                // the `done` event whose state the client based its
                // decisions on. Using a synthetic counter here would
                // make the compositor reject every apply with
                // `cancelled` because the serial would never match.
                state.flush_to_snapshot(serial);
            }
            zwlr_output_manager_v1::Event::Finished => {
                // Compositor going away; nothing useful to do
                // beyond clearing state. The connection will close
                // shortly after this event.
                state.staging_heads.clear();
                state.staging_modes.clear();
                state.head_handles.clear();
                state.mode_handles.clear();
            }
            _ => {}
        }
    }

    // The `head` event (opcode 0) constructs a new ZwlrOutputHeadV1
    // proxy. wayland-client requires us to declare which protocol
    // opcodes spawn child proxies so it can pre-allocate the proxy
    // and route subsequent events through the right Dispatch impl;
    // without this, dispatch panics on the first head event.
    event_created_child!(DispatchState, ZwlrOutputManagerV1, [
        0 => (ZwlrOutputHeadV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputHeadV1, ()> for DispatchState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let id = proxy.id().protocol_id();
        let entry = state.staging_heads.entry(id).or_default();
        match event {
            zwlr_output_head_v1::Event::Name { name } => {
                entry.name = name.clone();
                state.head_handles.insert(name, proxy.clone());
            }
            zwlr_output_head_v1::Event::Description { description } => {
                entry.description = description;
            }
            zwlr_output_head_v1::Event::PhysicalSize { width, height } => {
                entry.physical_size_mm = (width, height);
            }
            zwlr_output_head_v1::Event::Mode { mode } => {
                let mid = mode.id().protocol_id();
                entry.modes.push(mode);
                state.mode_to_head.insert(mid, id);
                state.staging_modes.entry(mid).or_default();
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => {
                entry.enabled = enabled != 0;
            }
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                entry.current_mode = Some(mode);
            }
            zwlr_output_head_v1::Event::Position { x, y } => {
                entry.position = Position { x, y };
            }
            zwlr_output_head_v1::Event::Transform { transform } => {
                if let WEnum::Value(t) = transform {
                    if let Some(ours) = Transform::from_wlr(t as u32) {
                        entry.transform = ours;
                    }
                }
            }
            zwlr_output_head_v1::Event::Scale { scale } => {
                entry.scale = scale;
            }
            zwlr_output_head_v1::Event::Make { make } => entry.make = make,
            zwlr_output_head_v1::Event::Model { model } => entry.model = model,
            zwlr_output_head_v1::Event::SerialNumber { serial_number } => {
                entry.serial = serial_number;
            }
            zwlr_output_head_v1::Event::AdaptiveSync { state: vrr_state } => {
                entry.adaptive_sync = matches!(vrr_state, WEnum::Value(AdaptiveSyncState::Enabled));
            }
            zwlr_output_head_v1::Event::Finished => {
                state.staging_heads.remove(&id);
                state.head_handles.retain(|_, h| h.id() != proxy.id());
            }
            _ => {}
        }
    }

    // The `mode` event (opcode 3 in v1) creates a new
    // ZwlrOutputModeV1 child proxy. Same reason as the manager's
    // head event — wayland-client wants the spec up front. Opcode
    // numbering: name=0, description=1, physical_size=2, mode=3.
    event_created_child!(DispatchState, ZwlrOutputHeadV1, [
        3 => (ZwlrOutputModeV1, ()),
    ]);
}

impl Dispatch<ZwlrOutputModeV1, ()> for DispatchState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let id = proxy.id().protocol_id();
        let entry = state.staging_modes.entry(id).or_default();
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                entry.width = width;
                entry.height = height;
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => {
                entry.refresh_mhz = refresh as u32;
            }
            zwlr_output_mode_v1::Event::Preferred => {
                entry.preferred = true;
            }
            zwlr_output_mode_v1::Event::Finished => {
                state.staging_modes.remove(&id);
                state.mode_to_head.remove(&id);
            }
            _ => {}
        }
    }
}

impl Dispatch<ZwlrOutputConfigurationV1, ApplyTag> for DispatchState {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        data: &ApplyTag,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        let pid = proxy.id().protocol_id();
        let request_id = state
            .pending_applies
            .remove(&pid)
            .unwrap_or_else(|| data.request_id.clone());
        let outcome = match event {
            zwlr_output_configuration_v1::Event::Succeeded => "succeeded",
            zwlr_output_configuration_v1::Event::Failed => "failed",
            zwlr_output_configuration_v1::Event::Cancelled => "cancelled",
            _ => return,
        };
        proxy.destroy();
        let _ = state.app.emit(
            "displays:apply-result",
            &serde_json::json!({
                "requestId": request_id,
                "outcome":   outcome,
            }),
        );
    }
}

impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for DispatchState {
    fn event(
        _state: &mut Self,
        _proxy: &ZwlrOutputConfigurationHeadV1,
        _event: <ZwlrOutputConfigurationHeadV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // No-op: the head-config proxy has no events of its own.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_helpers_are_consistent() {
        for raw in 0u32..8 {
            let t = Transform::from_wlr(raw).unwrap();
            assert_eq!(t.to_wlr(), raw);
        }
    }

    #[test]
    fn enabled_kind_serialises_with_tag() {
        let ek = EnabledKind::Mirror("DP-1".into());
        let json = serde_json::to_string(&ek).unwrap();
        assert!(json.contains("\"type\":\"mirror\""));
        assert!(json.contains("\"target\":\"DP-1\""));
    }

    fn mode(w: i32, h: i32, mhz: u32) -> MonitorMode {
        MonitorMode {
            width: w,
            height: h,
            refresh_mhz: mhz,
            preferred: false,
        }
    }

    #[test]
    fn mirror_mode_match_prefers_closest_refresh() {
        // Source has 60 and 24 Hz at the target resolution; target
        // runs 59.94 Hz. Closer-refresh wins, never the cinema mode.
        let modes = vec![
            mode(2560, 1440, 144_000),
            mode(1920, 1080, 24_000),
            mode(1920, 1080, 60_000),
            mode(1280, 720, 60_000),
        ];
        let target = mode(1920, 1080, 59_940);
        assert_eq!(match_mirror_mode(&modes, &target), Some(2));
    }

    #[test]
    fn mirror_mode_match_returns_none_when_no_dimension_match() {
        let modes = vec![mode(2560, 1440, 60_000), mode(1280, 720, 60_000)];
        let target = mode(1920, 1080, 60_000);
        assert_eq!(match_mirror_mode(&modes, &target), None);
    }

    #[test]
    fn mirror_mode_match_picks_only_dimension_match() {
        // Single matching dimension wins regardless of refresh skew.
        let modes = vec![mode(2560, 1440, 60_000), mode(1920, 1080, 30_000)];
        let target = mode(1920, 1080, 60_000);
        assert_eq!(match_mirror_mode(&modes, &target), Some(1));
    }
}
