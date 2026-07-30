//! SX-5: the one list of everything that extends the system.
//!
//! Apps, modules and bridges answered separately produce three surfaces that
//! each look complete and none of which is. This is the single query behind
//! the unified view: the shared inventory read from disk, with the module
//! runtime's live answer laid over the modules it knows about.
//!
//! The capability labels come from `arlen-extensions`, which is also what the
//! store uses, so a filter for "things that can reach the internet" means the
//! same thing on both surfaces. That is the whole reason the vocabulary is
//! shared rather than written per view.

use arlen_extensions::inventory::{self, InventoryRoots, LiveModule};
use arlen_extensions::Extension;
use modulesd_proto::{client, Request, Response};

/// Everything installed, with what it was granted and what it is doing.
///
/// The disk read always succeeds - an absent source means that kind is simply
/// empty. The runtime overlay is best-effort on top: if modulesd is not
/// running, modules keep the `unknown` health the disk read gave them rather
/// than the whole call failing. A management surface that goes blank exactly
/// when a daemon is down fails at the moment it is needed.
#[tauri::command]
pub async fn extensions_list() -> Result<Vec<Extension>, String> {
    let roots = InventoryRoots {
        disabled_modules: disabled_modules(),
        ..Default::default()
    };
    let mut rows = inventory::read(&roots);
    if let Some(live) = live_modules().await {
        inventory::overlay_modules(&mut rows, &live);
    }
    Ok(rows)
}

/// The user's disabled list, so a module reads as switched off rather than
/// unknown even when the runtime cannot be reached.
fn disabled_modules() -> std::collections::BTreeSet<String> {
    super::modules::modules_list()
        .into_iter()
        .filter(|m| !m.enabled)
        .map(|m| m.id)
        .collect()
}

/// Ask the runtime what it currently holds, or `None` if it is not reachable.
async fn live_modules() -> Option<Vec<LiveModule>> {
    let request = Request::ListModules {
        id: "extensions-list".to_string(),
    };
    match client::request_once(&client::socket_path(), request).await {
        Ok(Response::ModuleList { modules, .. }) => Some(
            modules
                .into_iter()
                .map(|m| LiveModule {
                    id: m.id,
                    enabled: m.enabled,
                    failed: m.failed,
                    last_error: m.last_error,
                })
                .collect(),
        ),
        // Anything else - unreachable, an error reply, a reply we did not
        // expect - leaves the disk answer standing. It is less informative,
        // never wrong.
        _ => None,
    }
}
