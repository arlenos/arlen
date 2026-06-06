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
    /// True when `zcosmic_toplevel_handle_v1::State::Minimized` is set.
    /// The cosmic-toplevel-info protocol broadcasts this on every state
    /// transition, so the shell's minimized-windows UI reads state
    /// here rather than subscribing to separate event-bus events.
    #[serde(default)]
    pub minimized: bool,
    /// True when `zcosmic_toplevel_handle_v1::State::Fullscreen` is
    /// set. Read by the overlay's context menu so the "Fullscreen"
    /// entry can toggle correctly (send `unset_fullscreen` when the
    /// window is already fullscreen).
    #[serde(default)]
    pub fullscreen: bool,
    pub workspace_ids: Vec<String>,
    /// Output connectors (`DP-1`, `HDMI-A-1`, …) the toplevel is
    /// currently visible on. Multi-output windows (sticky, or
    /// stretching across two monitors) appear in more than one
    /// entry. Used by the per-output GlobalMenuBar so each bar
    /// shows the menu only when this window lives on its monitor.
    #[serde(default)]
    pub output_connectors: Vec<String>,
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
    /// Output connectors (`DP-1`, `HDMI-A-1`, …) this workspace's
    /// group spans. Per-output WorkspaceIndicator filters its
    /// strip on this list so each bar shows only its own
    /// monitor's workspaces.
    #[serde(default)]
    pub output_connectors: Vec<String>,
}

/// Shared snapshot of the latest workspace list. Written every time
/// the compositor fires `WorkspaceHandler::done`; read by the
/// `get_workspaces` Tauri command so the Svelte frontend can prime
/// its store on mount / after a HMR full-page reload. Without this
/// cache the WorkspaceIndicator stayed hidden after any Vite page
/// reload because the compositor only emits `workspace-list` on
/// actual state changes, not on request.
pub type WorkspaceList = Arc<Mutex<Vec<WorkspaceInfo>>>;

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

/// Shared state for activating / moving windows from any thread.
pub struct ToplevelSender {
    /// Toplevel cosmic handles keyed by identifier string.
    handles: Mutex<HashMap<String, zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1>>,
    /// The toplevel manager proxy.
    manager: Mutex<Option<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1>>,
    /// The first bound wl_seat.
    seat: Mutex<Option<wl_seat::WlSeat>>,
    /// Connection for flushing.
    conn: Mutex<Option<Connection>>,
    /// Workspace handles keyed by workspace id (same encoding as
    /// `WorkspaceInfo.id`). Needed for `move_to_ext_workspace` — the
    /// request takes the destination workspace + its output, which we
    /// don't have access to from the Tauri command call site. The
    /// caches are rebuilt every time the compositor fires
    /// `WorkspaceHandler::done`, so they always reflect the live
    /// compositor view without requiring a read lock on AppData.
    workspace_handles:
        Mutex<HashMap<String, ext_workspace_handle_v1::ExtWorkspaceHandleV1>>,
    /// First output of the workspace's group, keyed by workspace id.
    /// One output per workspace is sufficient because lunaris currently
    /// assumes workspaces belong to exactly one output (single-output
    /// group model). Multi-output groups would need a different policy.
    workspace_outputs: Mutex<HashMap<String, wl_output::WlOutput>>,
}

impl ToplevelSender {
    /// Creates an empty sender.
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(HashMap::new()),
            manager: Mutex::new(None),
            seat: Mutex::new(None),
            conn: Mutex::new(None),
            workspace_handles: Mutex::new(HashMap::new()),
            workspace_outputs: Mutex::new(HashMap::new()),
        }
    }

    /// Ask the compositor to close the window via
    /// `zcosmic_toplevel_manager_v1::close`. The target app sees a
    /// polite close request and can prompt for unsaved-work confirm
    /// — this is the equivalent of a titlebar × button click, not a
    /// SIGKILL. No-op on missing handle / manager.
    pub fn close(&self, id: &str) {
        let handles = self.handles.lock().unwrap();
        let Some(handle) = handles.get(id) else {
            log::warn!("toplevel_sender: close: no handle for id={id}");
            return;
        };
        let manager = self.manager.lock().unwrap();
        let Some(mgr) = manager.as_ref() else {
            log::warn!("toplevel_sender: close: no manager bound");
            return;
        };
        mgr.close(handle);
        drop(handles);
        drop(manager);
        self.flush();
        log::debug!("toplevel_sender: close id={id}");
    }

    /// Toggle the fullscreen state of a window via cosmic-toplevel-
    /// management. `enable=true` sends `set_fullscreen` (with no
    /// output override so the compositor picks the window's current
    /// output); `false` sends `unset_fullscreen`.
    pub fn set_fullscreen(&self, id: &str, enable: bool) {
        let handles = self.handles.lock().unwrap();
        let Some(handle) = handles.get(id) else {
            log::warn!("toplevel_sender: set_fullscreen: no handle for id={id}");
            return;
        };
        let manager = self.manager.lock().unwrap();
        let Some(mgr) = manager.as_ref() else {
            log::warn!("toplevel_sender: set_fullscreen: no manager bound");
            return;
        };
        if enable {
            // `set_fullscreen` signature: (toplevel, Option<output>).
            // Passing None lets the compositor keep the toplevel's
            // current output, which is what a user toggling "go
            // fullscreen" from the overlay expects.
            mgr.set_fullscreen(handle, None);
        } else {
            mgr.unset_fullscreen(handle);
        }
        drop(handles);
        drop(manager);
        self.flush();
        log::debug!("toplevel_sender: set_fullscreen({enable}) id={id}");
    }

    /// Toggle the minimized state of a window via cosmic-toplevel-
    /// management. `minimize=true` hides, `minimize=false` restores
    /// without focusing — call `activate()` separately if the caller
    /// wants follow-focus.
    pub fn set_minimized(&self, id: &str, minimize: bool) {
        let handles = self.handles.lock().unwrap();
        let Some(handle) = handles.get(id) else {
            log::warn!("toplevel_sender: set_minimized: no handle for id={id}");
            return;
        };
        let manager = self.manager.lock().unwrap();
        let Some(mgr) = manager.as_ref() else {
            log::warn!("toplevel_sender: set_minimized: no manager bound");
            return;
        };
        if minimize {
            mgr.set_minimized(handle);
        } else {
            mgr.unset_minimized(handle);
        }
        drop(handles);
        drop(manager);
        self.flush();
        log::debug!("toplevel_sender: set_minimized({minimize}) id={id}");
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

    /// Sends `move_to_ext_workspace` for the given window + destination.
    /// No-ops with a warn log if any handle is missing — the UI should
    /// always call this with live ids from the Svelte stores, so a miss
    /// means the window/workspace disappeared between the drag start
    /// and the drop (benign). Does not auto-focus the window; call
    /// `activate` separately if follow-focus is desired.
    pub fn move_to_workspace(&self, window_id: &str, workspace_id: &str) {
        let handles = self.handles.lock().unwrap();
        let Some(toplevel) = handles.get(window_id) else {
            log::warn!(
                "toplevel_sender: move_to_workspace: no toplevel for window id={window_id}"
            );
            return;
        };
        let manager = self.manager.lock().unwrap();
        let Some(mgr) = manager.as_ref() else {
            log::warn!("toplevel_sender: move_to_workspace: no manager bound");
            return;
        };
        let ws_handles = self.workspace_handles.lock().unwrap();
        let Some(workspace) = ws_handles.get(workspace_id) else {
            log::warn!(
                "toplevel_sender: move_to_workspace: no workspace handle for \
                 id={workspace_id}"
            );
            return;
        };
        let outputs = self.workspace_outputs.lock().unwrap();
        let Some(output) = outputs.get(workspace_id) else {
            log::warn!(
                "toplevel_sender: move_to_workspace: no output for workspace \
                 id={workspace_id} (workspace group has no output assigned?)"
            );
            return;
        };

        mgr.move_to_ext_workspace(toplevel, workspace, output);
        drop(handles);
        drop(manager);
        drop(ws_handles);
        drop(outputs);
        self.flush();
        log::info!(
            "toplevel_sender: move_to_workspace window={window_id} \
             workspace={workspace_id}"
        );
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
    workspace_list: WorkspaceList,
    output_state: OutputState,
    registry_state: RegistryState,
    toplevel_info_state: ToplevelInfoState,
    toplevel_manager_state: Option<ToplevelManagerState>,
    workspace_state: WorkspaceState,
    /// Registry shared with `output_bars`. We trigger a refresh
    /// from this thread whenever the xdg-output table changes so
    /// the per-bar `connector` field gets backfilled without
    /// waiting for a hot-plug event.
    output_bar_registry: Arc<crate::output_bars::OutputBarRegistry>,
    /// Cache of `(geometry origin, connector)` tuples consumed by
    /// `output_bars::sync_bars` to map each `gdk::Monitor` to its
    /// connector name.
    output_connector_table: Arc<crate::output_bars::OutputConnectorTable>,
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
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish_output_table();
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {
        self.publish_output_table();
    }
    fn output_destroyed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_output::WlOutput,
    ) {
        self.publish_output_table();
    }
}

impl AppData {
    /// Snapshot every known wl_output's logical position +
    /// connector and write it to the shared
    /// `OutputConnectorTable`. Then schedule an `output_bars`
    /// refresh so the per-bar `connector` field gets backfilled,
    /// AND re-emit the current toplevel + workspace snapshots so
    /// frontends that already consumed payloads with empty
    /// `output_connectors` (race against xdg-output name arrival)
    /// receive the resolved values without waiting for an
    /// unrelated workspace/window event.
    fn publish_output_table(&mut self) {
        use crate::output_bars::OutputGeometry;
        let table: Vec<OutputGeometry> = self
            .output_state
            .outputs()
            .filter_map(|o| {
                let info = self.output_state.info(&o)?;
                let connector = info.name?;
                let (x, y) = info.logical_position?;
                Some(OutputGeometry { x, y, connector })
            })
            .collect();
        log::debug!(
            "wayland_client: output table now has {} entries",
            table.len()
        );
        self.output_connector_table.update(table);
        crate::output_bars::refresh(
            self.app_handle.clone(),
            Arc::clone(&self.output_bar_registry),
            Arc::clone(&self.output_connector_table),
        );

        self.republish_toplevels();
        self.republish_workspaces();
    }

    /// Re-emit `lunaris://toplevel-changed` for every known
    /// toplevel. The freshly-built payloads carry the now-resolved
    /// `output_connectors` so the frontend's per-output
    /// `activeWindowForOutput` filter starts seeing the correct
    /// monitor assignment.
    fn republish_toplevels(&self) {
        let snapshots: Vec<ToplevelPayload> = self
            .toplevel_info_state
            .toplevels()
            .map(|info| ToplevelPayload {
                id: info.identifier.clone(),
                title: info.title.clone(),
                app_id: info.app_id.clone(),
                active: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Activated),
                minimized: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Minimized),
                fullscreen: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Fullscreen),
                workspace_ids: info
                    .workspace
                    .iter()
                    .map(|h| h.id().to_string())
                    .collect(),
                output_connectors: self.resolve_connectors(info.output.iter()),
            })
            .collect();
        for payload in snapshots {
            {
                let mut wl = self.window_list.lock().unwrap();
                if let Some(pos) = wl.iter().position(|w| w.id == payload.id) {
                    wl[pos] = payload.clone();
                }
            }
            let _ = self
                .app_handle
                .emit("lunaris://toplevel-changed", payload);
        }
    }

    /// Recompute and re-emit `lunaris://workspace-list` from the
    /// current `workspace_state`. Mirrors the work
    /// `WorkspaceHandler::done` does on every compositor batch,
    /// minus the manager-handle bookkeeping (unchanged here).
    fn republish_workspaces(&mut self) {
        let mut infos: Vec<WorkspaceInfo> = Vec::new();
        for group in self.workspace_state.workspace_groups() {
            let group_id = group.handle.id().to_string();
            let group_connectors = self.resolve_connectors(group.outputs.iter());

            let mut group_ws: Vec<_> = group
                .workspaces
                .iter()
                .filter_map(|wh| self.workspace_state.workspace_info(wh))
                .collect();
            group_ws.sort_by(|a, b| a.coordinates.cmp(&b.coordinates));

            for w in &group_ws {
                let active = w.state.contains(ext_workspace_handle_v1::State::Active);
                infos.push(WorkspaceInfo {
                    id: w.handle.id().to_string(),
                    group_id: group_id.clone(),
                    name: w.name.clone(),
                    active,
                    output_connectors: group_connectors.clone(),
                });
            }
        }
        *self.workspace_list.lock().unwrap() = infos.clone();
        let _ = self.app_handle.emit("lunaris://workspace-list", infos);
    }
}

impl AppData {
    /// Resolve a slice of `wl_output` proxies to their connector
    /// names ("DP-1", "HDMI-A-1") via the SCTK `OutputState`'s
    /// xdg-output cache. Outputs whose `name` hasn't arrived yet
    /// (transient binding race) are skipped — the next update
    /// event re-runs this and fills them in.
    fn resolve_connectors<'a, I>(&self, outputs: I) -> Vec<String>
    where
        I: IntoIterator<Item = &'a wl_output::WlOutput>,
    {
        let mut out: Vec<String> = outputs
            .into_iter()
            .filter_map(|o| self.output_state.info(o).and_then(|i| i.name))
            .collect();
        out.sort();
        out.dedup();
        out
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
                minimized: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Minimized),
                fullscreen: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Fullscreen),
                workspace_ids: info.workspace.iter().map(|h| h.id().to_string()).collect(),
                output_connectors: self.resolve_connectors(info.output.iter()),
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
                minimized: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Minimized),
                fullscreen: info
                    .state
                    .contains(&zcosmic_toplevel_handle_v1::State::Fullscreen),
                workspace_ids: info.workspace.iter().map(|h| h.id().to_string()).collect(),
                output_connectors: self.resolve_connectors(info.output.iter()),
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
        // Caches for the toplevel-manager's move_to_ext_workspace call.
        // Every workspace's entry gets a clone of its group's first
        // output so the Tauri command can stay output-agnostic.
        let mut toplevel_ws_handles: HashMap<
            String,
            ext_workspace_handle_v1::ExtWorkspaceHandleV1,
        > = HashMap::new();
        let mut toplevel_ws_outputs: HashMap<String, wl_output::WlOutput> = HashMap::new();

        for group in self.workspace_state.workspace_groups() {
            let group_id = group.handle.id().to_string();
            let primary_output = group.outputs.first().cloned();
            let group_connectors = self.resolve_connectors(group.outputs.iter());

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
                toplevel_ws_handles.insert(ws_id.clone(), w.handle.clone());
                if let Some(output) = primary_output.as_ref() {
                    toplevel_ws_outputs.insert(ws_id.clone(), output.clone());
                }
                infos.push(WorkspaceInfo {
                    id: ws_id,
                    group_id: group_id.clone(),
                    name: w.name.clone(),
                    active,
                    output_connectors: group_connectors.clone(),
                });
            }
        }

        *self.workspace_sender.handles.lock().unwrap() = handle_map;
        *self.toplevel_sender.workspace_handles.lock().unwrap() = toplevel_ws_handles;
        *self.toplevel_sender.workspace_outputs.lock().unwrap() = toplevel_ws_outputs;

        if let Ok(m) = self.workspace_state.workspace_manager().get() {
            *self.workspace_sender.manager.lock().unwrap() = Some(m.clone());
        }

        // Keep the shared cache in lock-step with the event so the
        // `get_workspaces` command always returns what the compositor
        // most recently told us. Cloned before emit so we can hand
        // both consumers the same snapshot.
        *self.workspace_list.lock().unwrap() = infos.clone();

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
    workspace_list: WorkspaceList,
    output_bar_registry: Arc<crate::output_bars::OutputBarRegistry>,
    output_connector_table: Arc<crate::output_bars::OutputConnectorTable>,
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
            workspace_list,
            output_state: OutputState::new(&globals, &qh),
            toplevel_info_state,
            toplevel_manager_state,
            workspace_state,
            registry_state,
            output_bar_registry,
            output_connector_table,
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

/// Moves the window identified by `window_id` to the workspace
/// identified by `target_workspace_id`. Used by the Workspace Overview
/// overlay's drag-and-drop. Does not shift keyboard focus — the shell
/// can call `activate_window` separately if follow-focus is desired.
#[tauri::command]
pub fn window_move_to_workspace(
    state: tauri::State<Arc<ToplevelSender>>,
    window_id: String,
    target_workspace_id: String,
) {
    state.move_to_workspace(&window_id, &target_workspace_id);
}

/// Returns the current list of open windows.
#[tauri::command]
pub fn get_windows(state: tauri::State<WindowList>) -> Vec<ToplevelPayload> {
    state.lock().unwrap().clone()
}

/// Returns the current list of workspaces. Called by the Svelte
/// `initWorkspaceListeners` to prime the store on mount — the
/// compositor only emits `lunaris://workspace-list` on state
/// changes, so without this priming call a HMR full-page reload
/// would leave the frontend store stuck at `[]` until the user
/// did something that caused the compositor to re-emit (switch
/// workspace, move a window, etc.).
#[tauri::command]
pub fn get_workspaces(state: tauri::State<WorkspaceList>) -> Vec<WorkspaceInfo> {
    state.lock().unwrap().clone()
}
