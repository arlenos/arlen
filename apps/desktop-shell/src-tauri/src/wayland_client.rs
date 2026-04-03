use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cosmic_client_toolkit::{
    cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1,
    sctk,
    sctk::{
        output::{OutputHandler, OutputState},
        registry::{ProvidesRegistryState, RegistryState},
    },
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
    workspace::{WorkspaceHandler, WorkspaceState},
    GlobalData,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use wayland_client::{
    globals::registry_queue_init, protocol::wl_output, Connection, Proxy, QueueHandle,
};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1;
use wayland_protocols::ext::workspace::v1::client::{
    ext_workspace_handle_v1, ext_workspace_manager_v1,
};

// ── Toplevel payloads ─────────────────────────────────────────────────────────

#[derive(Clone, Serialize)]
struct ToplevelPayload {
    id: String,
    title: String,
    app_id: String,
    active: bool,
    /// Workspace handle IDs this toplevel belongs to (usually one).
    workspace_ids: Vec<String>,
}

#[derive(Clone, Serialize)]
struct ToplevelRemovedPayload {
    id: String,
}

// ── Workspace payloads ────────────────────────────────────────────────────────

/// A single workspace entry emitted as part of `lunaris://workspace-list`.
#[derive(Clone, Serialize)]
pub struct WorkspaceInfo {
    /// Stable string ID derived from the Wayland object ID.
    pub id: String,
    /// Human-readable name as set by the compositor.
    pub name: String,
    /// Whether this workspace is currently active (visible on any output).
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

// ── Wayland app state ─────────────────────────────────────────────────────────

struct AppData {
    app_handle: AppHandle,
    workspace_sender: Arc<WorkspaceSender>,
    output_state: OutputState,
    registry_state: RegistryState,
    toplevel_info_state: ToplevelInfoState,
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
            let payload = ToplevelPayload {
                id: info.identifier.clone(),
                title: info.title.clone(),
                app_id: info.app_id.clone(),
                active: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Activated),
                workspace_ids: info.workspace.iter().map(|h| h.id().to_string()).collect(),
            };
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
            let payload = ToplevelPayload {
                id: info.identifier.clone(),
                title: info.title.clone(),
                app_id: info.app_id.clone(),
                active: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Activated),
                workspace_ids: info.workspace.iter().map(|h| h.id().to_string()).collect(),
            };
            let _ = self.app_handle.emit("lunaris://toplevel-changed", payload);
        }
    }

    fn toplevel_closed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        toplevel: &ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1,
    ) {
        // info() is still valid here - removal happens after this callback
        if let Some(info) = self.toplevel_info_state.info(toplevel) {
            let payload = ToplevelRemovedPayload {
                id: info.identifier.clone(),
            };
            let _ = self.app_handle.emit("lunaris://toplevel-removed", payload);
        }
    }
}

impl WorkspaceHandler for AppData {
    fn workspace_state(&mut self) -> &mut WorkspaceState {
        &mut self.workspace_state
    }

    /// Called after all pending workspace updates have been committed. Emits a
    /// full `lunaris://workspace-list` snapshot sorted by coordinates.
    fn done(&mut self) {
        let mut workspaces: Vec<_> = self.workspace_state.workspaces().collect();
        // Sort by coordinates so the order matches the compositor's layout
        // (first coordinate is the linear workspace index for 1-D workspaces).
        workspaces.sort_by(|a, b| a.coordinates.cmp(&b.coordinates));

        let infos: Vec<WorkspaceInfo> = workspaces
            .iter()
            .map(|w| WorkspaceInfo {
                id: w.handle.id().to_string(),
                name: w.name.clone(),
                active: w.state.contains(ext_workspace_handle_v1::State::Active),
            })
            .collect();

        // Refresh the handle map used by workspace_activate.
        *self.workspace_sender.handles.lock().unwrap() = workspaces
            .iter()
            .map(|w| (w.handle.id().to_string(), w.handle.clone()))
            .collect();

        // Keep the manager reference up to date.
        if let Ok(m) = self.workspace_state.workspace_manager().get() {
            *self.workspace_sender.manager.lock().unwrap() = Some(m.clone());
        }

        if let Err(e) = self.app_handle.emit("lunaris://workspace-list", infos) {
            log::error!("wayland_client: workspace-list emit failed: {e}");
        }
    }
}

// ── Delegate macros ───────────────────────────────────────────────────────────

sctk::delegate_output!(AppData);
sctk::delegate_registry!(AppData);
cosmic_client_toolkit::delegate_toplevel_info!(AppData);
cosmic_client_toolkit::delegate_workspace!(AppData);

// ── Thread entry point ────────────────────────────────────────────────────────

/// Spawns the Wayland client thread. Connects to the compositor, registers
/// toplevel and workspace handlers, and starts the event dispatch loop.
pub fn start(app_handle: AppHandle, workspace_sender: Arc<WorkspaceSender>) {
    std::thread::spawn(move || {
        // Retry loop: compositor socket may not be ready immediately.
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

        let workspace_state = WorkspaceState::new(&registry_state, &qh);

        // Move the connection into the sender so Tauri commands can call flush().
        // The event queue holds its own Arc on the connection internals and
        // continues to work after the move.
        *workspace_sender.conn.lock().unwrap() = Some(conn);

        let mut app_data = AppData {
            app_handle,
            workspace_sender,
            output_state: OutputState::new(&globals, &qh),
            toplevel_info_state,
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

// ── Tauri command ─────────────────────────────────────────────────────────────

/// Activates the workspace with the given ID. Called by the workspace indicator
/// when the user clicks a pill, dot, or the text display.
#[tauri::command]
pub fn workspace_activate(
    state: tauri::State<Arc<WorkspaceSender>>,
    id: String,
) {
    state.activate(&id);
}
