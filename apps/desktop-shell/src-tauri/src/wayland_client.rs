use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cosmic_client_toolkit::{
    cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1,
    cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1,
    sctk,
    sctk::{
        output::{OutputHandler, OutputState},
        registry::{ProvidesRegistryState, RegistryState},
    },
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
    toplevel_management::{ToplevelManagerHandler, ToplevelManagerState},
    workspace::{WorkspaceHandler, WorkspaceState},
    GlobalData,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use wayland_client::{
    globals::registry_queue_init, protocol::wl_output, protocol::wl_seat, Connection, Dispatch,
    Proxy, QueueHandle, WEnum,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1;
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_handle_v1, ext_workspace_manager_v1,
};

// ── Toplevel payloads ─────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
pub struct ToplevelPayload {
    pub id: String,
    pub title: String,
    pub app_id: String,
    pub active: bool,
    pub workspace_ids: Vec<String>,
}

#[derive(Clone, Serialize)]
struct ToplevelRemovedPayload {
    id: String,
}

/// Shared snapshot of all known toplevels, updated on every event.
pub type WindowList = Arc<Mutex<Vec<ToplevelPayload>>>;

// ── Workspace payloads ────────────────────────────────────────────────────────

/// A single workspace entry emitted as part of `lunaris://workspace-list`.
#[derive(Clone, Serialize)]
pub struct WorkspaceInfo {
    /// Stable string ID derived from the Wayland object ID.
    pub id: String,
    /// ID of the workspace group this workspace belongs to.
    pub group_id: String,
    /// Human-readable name as set by the compositor.
    pub name: String,
    /// Whether this workspace is currently active on its output.
    pub active: bool,
}

// ── Shared back-channel (WorkspaceSender) ────────────────────────────────────

/// Shared state that lets Tauri commands send workspace requests back to the
/// compositor from any thread, using the same pattern as `ShellOverlaySender`.
pub struct WorkspaceSender {
    handles: Mutex<HashMap<String, ext_workspace_handle_v1::ExtWorkspaceHandleV1>>,
    manager: Mutex<Option<ext_workspace_manager_v1::ExtWorkspaceManagerV1>>,
    conn: Mutex<Option<Connection>>,
}

impl WorkspaceSender {
    /// Creates an empty sender. Fields are populated once the Wayland client
    /// thread successfully binds the workspace globals.
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
            manager: Mutex::new(None),
            conn: Mutex::new(None),
        }
    }

    /// Activates the workspace identified by `id` and flushes immediately.
    pub fn activate(&self, id: &str) {
        let handles = self.handles.lock().unwrap();
        if let Some(handle) = handles.get(id) {
            handle.activate();
            drop(handles);
            if let Some(m) = self.manager.lock().unwrap().as_ref() {
                m.commit();
            }
            self.flush();
        }
    }

    fn flush(&self) {
        if let Some(c) = self.conn.lock().unwrap().as_ref() {
            if let Err(e) = c.flush() {
                log::warn!("wayland_client: workspace flush failed: {e}");
            }
        }
    }
}

// ── Shared back-channel (ToplevelSender) ─────────────────────────────────────

/// Shared state for activating windows from any thread.
pub struct ToplevelSender {
    /// Toplevel cosmic handles keyed by identifier string.
    handles: Mutex<HashMap<String, zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1>>,
    /// The toplevel manager proxy.
    manager: Mutex<Option<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1>>,
    /// The first bound wl_seat.
    seat: Mutex<Option<wl_seat::WlSeat>>,
    /// Connection for flushing.
    conn: Mutex<Option<Connection>>,
}

impl ToplevelSender {
    /// Creates an empty sender.
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
            manager: Mutex::new(None),
            seat: Mutex::new(None),
            conn: Mutex::new(None),
        }
    }

    /// Activates (focuses) the window with the given identifier.
    pub fn activate(&self, id: &str) {
        let handles = self.handles.lock().unwrap();
        let Some(handle) = handles.get(id) else {
            log::warn!("toplevel_sender: no handle for id={id}");
            return;
        };
        let manager = self.manager.lock().unwrap();
        let Some(mgr) = manager.as_ref() else {
            log::warn!("toplevel_sender: no manager bound");
            return;
        };
        let seat = self.seat.lock().unwrap();
        let Some(s) = seat.as_ref() else {
            log::warn!("toplevel_sender: no seat bound");
            return;
        };
        mgr.activate(handle, s);
        drop(handles);
        drop(manager);
        drop(seat);
        self.flush();
        log::info!("toplevel_sender: activated window id={id}");
    }

    fn flush(&self) {
        if let Some(c) = self.conn.lock().unwrap().as_ref() {
            if let Err(e) = c.flush() {
                log::warn!("wayland_client: toplevel flush failed: {e}");
            }
        }
    }
}

// ── Wayland app state ─────────────────────────────────────────────────────────

struct AppData {
    app_handle: AppHandle,
    workspace_sender: Arc<WorkspaceSender>,
    toplevel_sender: Arc<ToplevelSender>,
    window_list: WindowList,
    output_state: OutputState,
    registry_state: RegistryState,
    toplevel_info_state: ToplevelInfoState,
    toplevel_manager_state: Option<ToplevelManagerState>,
    workspace_state: WorkspaceState,
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    sctk::registry_handlers!(OutputState);
}

impl OutputHandler for AppData {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_output::WlOutput,
    ) {
    }
}

impl ToplevelInfoHandler for AppData {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }

    fn new_toplevel(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        toplevel: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    ) {
        if let Some(info) = self.toplevel_info_state.info(toplevel) {
            log::info!(
                "wayland_client: new_toplevel id={} title={:?} app_id={:?} has_cosmic={}",
                info.identifier, info.title, info.app_id, info.cosmic_toplevel.is_some(),
            );
            // Store the cosmic handle for window activation.
            if let Some(cosmic_handle) = &info.cosmic_toplevel {
                self.toplevel_sender
                    .handles
                    .lock()
                    .unwrap()
                    .insert(info.identifier.clone(), cosmic_handle.clone());
            }

            let payload = ToplevelPayload {
                id: info.identifier.clone(),
                title: info.title.clone(),
                app_id: info.app_id.clone(),
                active: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Activated),
                workspace_ids: info.workspace.iter().map(|h| h.id().to_string()).collect(),
            };
            {
                let mut wl = self.window_list.lock().unwrap();
                wl.retain(|w| w.id != payload.id);
                wl.push(payload.clone());
            }
            let _ = self.app_handle.emit("lunaris://toplevel-added", payload);
        }
    }

    fn update_toplevel(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        toplevel: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    ) {
        if let Some(info) = self.toplevel_info_state.info(toplevel) {
            // Update the cosmic handle (may change between updates).
            if let Some(cosmic_handle) = &info.cosmic_toplevel {
                self.toplevel_sender
                    .handles
                    .lock()
                    .unwrap()
                    .insert(info.identifier.clone(), cosmic_handle.clone());
            }

            let payload = ToplevelPayload {
                id: info.identifier.clone(),
                title: info.title.clone(),
                app_id: info.app_id.clone(),
                active: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Activated),
                workspace_ids: info.workspace.iter().map(|h| h.id().to_string()).collect(),
            };
            {
                let mut wl = self.window_list.lock().unwrap();
                if let Some(pos) = wl.iter().position(|w| w.id == payload.id) {
                    wl[pos] = payload.clone();
                }
            }
            let _ = self.app_handle.emit("lunaris://toplevel-changed", payload);
        }
    }

    fn toplevel_closed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        toplevel: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    ) {
        if let Some(info) = self.toplevel_info_state.info(toplevel) {
            // Remove the handle.
            self.toplevel_sender
                .handles
                .lock()
                .unwrap()
                .remove(&info.identifier);

            let payload = ToplevelRemovedPayload {
                id: info.identifier.clone(),
            };
            self.window_list.lock().unwrap().retain(|w| w.id != payload.id);
            let _ = self.app_handle.emit("lunaris://toplevel-removed", payload);
        }
    }
}

impl ToplevelManagerHandler for AppData {
    fn toplevel_manager_state(&mut self) -> &mut ToplevelManagerState {
        self.toplevel_manager_state.as_mut().unwrap()
    }

    fn capabilities(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        capabilities: Vec<
            WEnum<zcosmic_toplevel_manager_v1::ZcosmicToplelevelManagementCapabilitiesV1>,
        >,
    ) {
        let caps: Vec<String> = capabilities
            .iter()
            .filter_map(|c| match c {
                WEnum::Value(v) => Some(format!("{v:?}")),
                _ => None,
            })
            .collect();
        log::info!("wayland_client: toplevel manager capabilities: {caps:?}");
    }
}

impl WorkspaceHandler for AppData {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspace_state
    }

    fn done(&mut self) {
        let mut infos: Vec<WorkspaceInfo> = Vec::new();
        let mut handle_map: HashMap<String, ext_workspace_handle_v1::ExtWorkspaceHandleV1> =
            HashMap::new();

        for group in self.workspace_state.workspace_groups() {
            let group_id = group.handle.id().to_string();

            let mut group_ws: Vec<_> = group
                .workspaces
                .iter()
                .filter_map(|wh| self.workspace_state.workspace_info(wh))
                .collect();
            group_ws.sort_by(|a, b| a.coordinates.cmp(&b.coordinates));

            for w in &group_ws {
                let ws_id = w.handle.id().to_string();
                let active = w.state.contains(ext_workspace_handle_v1::State::Active);
                handle_map.insert(ws_id.clone(), w.handle.clone());
                infos.push(WorkspaceInfo {
                    id: ws_id,
                    group_id: group_id.clone(),
                    name: w.name.clone(),
                    active,
                });
            }
        }

        *self.workspace_sender.handles.lock().unwrap() = handle_map;

        if let Ok(m) = self.workspace_state.workspace_manager().get() {
            *self.workspace_sender.manager.lock().unwrap() = Some(m.clone());
        }

        if let Err(e) = self.app_handle.emit("lunaris://workspace-list", infos) {
            log::error!("wayland_client: workspace-list emit failed: {e}");
        }
    }
}

// ── Minimal wl_seat dispatch (we only need the object, not events) ───────────

impl Dispatch<wl_seat::WlSeat, ()> for AppData {
    fn event(
        _state: &mut Self,
        _proxy: &wl_seat::WlSeat,
        _event: wl_seat::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        // We only need the seat object for activate(). Ignore events.
    }
}

// ── Delegate macros ───────────────────────────────────────────────────────────

sctk::delegate_output!(AppData);
sctk::delegate_registry!(AppData);
cosmic_client_toolkit::delegate_toplevel_info!(AppData);
cosmic_client_toolkit::delegate_toplevel_manager!(AppData);
cosmic_client_toolkit::delegate_workspace!(AppData);

// ── Thread entry point ────────────────────────────────────────────────────────

/// Spawns the Wayland client thread. Connects to the compositor, registers
/// toplevel, workspace, and management handlers, then starts the event loop.
pub fn start(
    app_handle: AppHandle,
    workspace_sender: Arc<WorkspaceSender>,
    toplevel_sender: Arc<ToplevelSender>,
    window_list: WindowList,
) {
    std::thread::spawn(move || {
        let conn = loop {
            match Connection::connect_to_env() {
                Ok(c) => break c,
                Err(e) => {
                    log::debug!("wayland_client: not ready yet, retrying in 1s: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        };

        let (globals, mut event_queue) = match registry_queue_init::<AppData>(&conn) {
            Ok(r) => r,
            Err(e) => {
                log::error!("wayland_client: registry init failed: {e}");
                return;
            }
        };

        let qh = event_queue.handle();
        let registry_state = RegistryState::new(&globals);

        let toplevel_info_state = match ToplevelInfoState::try_new(&registry_state, &qh) {
            Some(s) => s,
            None => {
                log::error!(
                    "wayland_client: compositor does not expose ext-foreign-toplevel-list-v1"
                );
                return;
            }
        };

        let toplevel_manager_state = ToplevelManagerState::try_new(&registry_state, &qh);
        if toplevel_manager_state.is_some() {
            log::info!("wayland_client: toplevel manager bound");
        } else {
            log::warn!("wayland_client: toplevel manager not available (window activation disabled)");
        }

        // Bind the first wl_seat for activate() calls.
        let seat: Option<wl_seat::WlSeat> = globals
            .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=9, ())
            .ok();
        if seat.is_some() {
            log::info!("wayland_client: wl_seat bound");
        } else {
            log::warn!("wayland_client: no wl_seat found");
        }

        let workspace_state = WorkspaceState::new(&registry_state, &qh);

        // Share connection and manager with the toplevel sender.
        *toplevel_sender.conn.lock().unwrap() = Some(conn.clone());
        if let Some(ref mgr_state) = toplevel_manager_state {
            *toplevel_sender.manager.lock().unwrap() = Some(mgr_state.manager.clone());
        }
        if let Some(ref s) = seat {
            *toplevel_sender.seat.lock().unwrap() = Some(s.clone());
        }

        // Share connection with workspace sender.
        *workspace_sender.conn.lock().unwrap() = Some(conn);

        let mut app_data = AppData {
            app_handle,
            workspace_sender,
            toplevel_sender,
            window_list,
            output_state: OutputState::new(&globals, &qh),
            toplevel_info_state,
            toplevel_manager_state,
            workspace_state,
            registry_state,
        };

        loop {
            if let Err(e) = event_queue.blocking_dispatch(&mut app_data) {
                log::error!("wayland_client: dispatch error, thread exiting: {e}");
                break;
            }
        }
    });
}

// ── Tauri commands ───────────────────────────────────────────────────────────

/// Activates the workspace with the given ID.
#[tauri::command]
pub fn workspace_activate(state: tauri::State<Arc<WorkspaceSender>>, id: String) {
    state.activate(&id);
}

/// Activates (focuses) the window with the given identifier.
#[tauri::command]
pub fn activate_window(state: tauri::State<Arc<ToplevelSender>>, id: String) {
    state.activate(&id);
}

/// Returns the current list of open windows.
#[tauri::command]
pub fn get_windows(state: tauri::State<WindowList>) -> Vec<ToplevelPayload> {
    state.lock().unwrap().clone()
}
