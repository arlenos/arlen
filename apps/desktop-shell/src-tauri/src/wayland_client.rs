use cosmic_client_toolkit::{
    cosmic_protocols::toplevel_info::v1::client::zcosmic_toplevel_handle_v1,
    sctk::{
        output::{OutputHandler, OutputState},
        registry::{ProvidesRegistryState, RegistryState},
    },
    toplevel_info::{ToplevelInfoHandler, ToplevelInfoState},
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use wayland_client::{globals::registry_queue_init, protocol::wl_output, Connection, QueueHandle};
use wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1;

use cosmic_client_toolkit::sctk;

#[derive(Clone, Serialize)]
struct ToplevelPayload {
    id: String,
    title: String,
    app_id: String,
    active: bool,
}

#[derive(Clone, Serialize)]
struct ToplevelRemovedPayload {
    id: String,
}

struct AppData {
    app_handle: AppHandle,
    output_state: OutputState,
    registry_state: RegistryState,
    toplevel_info_state: ToplevelInfoState,
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

sctk::delegate_output!(AppData);
sctk::delegate_registry!(AppData);
cosmic_client_toolkit::delegate_toplevel_info!(AppData);

pub fn start(app_handle: AppHandle) {
    std::thread::spawn(move || {
        // Retry loop: compositor socket may not be ready immediately
        let conn = loop {
            match Connection::connect_to_env() {
                Ok(c) => break c,
                Err(e) => {
                    log::debug!("wayland_client: not ready yet, retrying in 1s: {e}");
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        };

        let (globals, mut event_queue) = match registry_queue_init(&conn) {
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

        let mut app_data = AppData {
            app_handle,
            output_state: OutputState::new(&globals, &qh),
            toplevel_info_state,
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
