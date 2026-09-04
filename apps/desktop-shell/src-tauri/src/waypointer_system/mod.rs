//! Waypointer plugin system (Phase 2: internal compiled plugins).
//!
//! Defines the `WaypointerPlugin` trait and `PluginManager` that aggregates
//! results from all registered plugins, sorted by relevance and priority.
//!
//! See `docs/architecture/waypointer-migration.md`.

mod plugin;
mod manager;
pub mod plugins;
pub mod registry;

pub use plugin::*;
pub use manager::*;

use std::sync::RwLock;

/// Tauri managed state for the plugin manager.
///
/// `RwLock` rather than `Mutex`: registration happens once at startup
/// (brief `.write()`), while `search` / `execute` are called on every
/// Waypointer keystroke from multiple plugins in parallel. Previously
/// a `Mutex` serialised every lookup, so a slow plugin (e.g. Files'
/// graph round-trip) blocked all other plugins' searches. Since
/// `search_plugin` takes `&self` under the hood — `WaypointerPlugin`
/// methods are immutable — concurrent reads are always safe.
pub type PluginManagerState = RwLock<PluginManager>;

/// Search via the plugin manager, plus whatever installed modules contribute.
///
/// The builtin results come first and are not made to wait on the module
/// runtime: modulesd is asked best-effort, and if it is down or slow the
/// launcher still answers with everything it can compute in-process. A
/// launcher that stalls because an extension host is wedged is worse than one
/// that shows fewer rows.
///
/// Module results are filtered through `module_results::accept`, which bounds
/// what a sandboxed module may ask the shell to do - notably it may not have
/// the shell run a command.
#[tauri::command]
pub async fn waypointer_search(
    query: String,
    state: tauri::State<'_, PluginManagerState>,
) -> Result<Vec<SearchResult>, String> {
    let mut results = {
        let mgr = state.read().unwrap();
        mgr.search(&query)
    };
    results.extend(module_results(&query).await);
    Ok(results)
}

/// What the installed modules found, or nothing if the runtime cannot answer.
async fn module_results(query: &str) -> Vec<SearchResult> {
    use arlen_desktop_shell_core::module_results;
    use modulesd_proto::{client, Request, Response};

    let request = Request::WaypointerSearchAll {
        id: "waypointer-search".to_string(),
        query: query.to_string(),
    };
    let reply = match client::request_once(&client::socket_path(), request).await {
        Ok(Response::WaypointerAggregate { results, .. }) => results,
        _ => return Vec::new(),
    };
    let (kept, dropped) = module_results::accept(reply);
    for (id, why) in dropped {
        // Logged rather than swallowed: a module whose results never appear is
        // otherwise indistinguishable from one that found nothing, which is a
        // miserable thing for its author to debug.
        log::debug!("waypointer: dropped module result {id}: {why:?}");
    }
    kept.iter().map(|e| e.to_search_result()).collect()
}

/// Execute a search result.
///
/// A builtin's result goes to the plugin manager, as it always has. A module's
/// cannot: its plugin id is prefixed `module:` so it cannot pose as a builtin,
/// and no builtin carries that id, so the manager would never find one. The
/// shell acts on the module's behalf instead, within the same closed set of
/// actions the search side allows, re-checked here because the result has been
/// out to the webview and back since then.
#[tauri::command]
pub fn waypointer_execute(
    result: SearchResult,
    state: tauri::State<'_, PluginManagerState>,
) -> Result<(), String> {
    use arlen_desktop_shell_core::module_results::{dispatch, Dispatch};

    match dispatch(&result) {
        Dispatch::Builtin => {
            let mgr = state.read().unwrap();
            mgr.execute(&result).map_err(|e| e.to_string())
        }
        Dispatch::Module(action) => run_for_module(&action),
        Dispatch::Refused(why) => {
            Err(format!("a module may not ask the shell to do this ({why:?})"))
        }
    }
}

/// Carry out the one action a module asked for, in the shell's own hands.
fn run_for_module(action: &arlen_desktop_shell_core::module_results::SafeAction) -> Result<(), String> {
    use arlen_desktop_shell_core::module_results::SafeAction;

    match action {
        SafeAction::Copy(text) => crate::clipboard_history::copy_via_wl_copy(text),
        SafeAction::OpenUrl(target) | SafeAction::OpenPath(target) => open_with_handler(target),
    }
}

/// Hand something to the desktop's own opener.
///
/// Through the shell's own launch path rather than `xdg-open`, so the shell's
/// launches are recorded and confined like everyone else's. They were the ones
/// that were not: the ledger this socket exists to fill had no line for anything
/// the waypointer opened, because the code that writes it sat behind a socket the
/// shell never dialled.
///
/// Blocking, because the callers are the synchronous plugin trait and a menu
/// action. The wait is one connection-free in-process call plus a spawn, which is
/// what the subprocess cost anyway.
pub fn open_with_handler(target: &str) -> Result<(), String> {
    use arlen_launch_contract as launch;
    // A path and a URI are different requests: the second is not a local file and
    // must not claim a path an application would then fail to open.
    let request = if target.starts_with('/') {
        launch::open_path_request(target)
    } else {
        launch::open_uri_request(target)
    };
    open_request(request)
}

/// Open an absolute path.
///
/// Separate from [`open_with_handler`] because the leading-slash guess is only
/// safe where the string genuinely could be either. A relative path handed to
/// another process means nothing - that process has its own working directory -
/// and silently treating one as a URI produces `file://notes.txt`, which resolves
/// to a host named `notes.txt` and opens nothing. Refusing says which it was.
pub fn open_path_with_handler(path: &str) -> Result<(), String> {
    if !path.starts_with('/') {
        return Err(format!("not an absolute path: {path}"));
    }
    open_request(arlen_launch_contract::open_path_request(path))
}

/// Open a URI.
pub fn open_uri_with_handler(uri: &str) -> Result<(), String> {
    open_request(arlen_launch_contract::open_uri_request(uri))
}

/// Put one request through the shell's own launch path and read the answer.
fn open_request(request: arlen_launch_contract::LaunchRequest) -> Result<(), String> {
    use arlen_launch_contract as launch;
    let outcome = tauri::async_runtime::block_on(crate::launch_service::dispatch(
        &request,
        &crate::launch_service::self_caller(),
    ));
    match outcome {
        launch::LaunchOutcome::Started { .. } => Ok(()),
        // The shell asked instead of opening: a dialog is on screen and the
        // person decides. Not an error to the caller - the request was taken.
        launch::LaunchOutcome::Asked { .. } => Ok(()),
        launch::LaunchOutcome::NoHandler { mime } => {
            Err(format!("nothing is set up to open {mime}"))
        }
        launch::LaunchOutcome::UnknownApplication { app_id } => {
            Err(format!("{app_id} is not installed"))
        }
        launch::LaunchOutcome::MalformedEntry { app_id, reason } => {
            Err(format!("{app_id} is installed wrong: {reason}"))
        }
        launch::LaunchOutcome::DidNotStart { app_id, reason } => {
            Err(format!("{app_id} did not start: {reason}"))
        }
        launch::LaunchOutcome::Refused => Err("the launch was refused".to_string()),
    }
}

/// List all currently-registered built-in plugins with their metadata.
/// The same data is written to the on-disk registry file at startup
/// (see `registry::write_registry`); this command is the in-process
/// equivalent used by the shell's own UI.
#[tauri::command]
pub fn waypointer_list_plugins(
    state: tauri::State<'_, PluginManagerState>,
) -> Vec<PluginDescriptor> {
    let mgr = state.read().unwrap();
    mgr.plugin_descriptors()
}

/// Query a single plugin by id. The Waypointer frontend uses this to
/// surface dedicated plugins (e.g. `core.power`) as their own
/// CommandGroup sections without routing through the generic search —
/// see `search_plugin` on `PluginManager` for why that matters.
#[tauri::command]
pub fn waypointer_search_plugin(
    plugin_id: String,
    query: String,
    state: tauri::State<'_, PluginManagerState>,
) -> Vec<SearchResult> {
    // DEBUG level: this fires per keystroke per registered plugin
    // group (~4 calls per key). Keeping it at info was ~80% of the
    // shell log noise during a short Waypointer session.
    log::debug!("waypointer_search_plugin: plugin_id='{plugin_id}' query='{query}'");
    let mgr = state.read().unwrap();
    let results = mgr.search_plugin(&plugin_id, &query);
    log::debug!(
        "waypointer_search_plugin: plugin_id='{plugin_id}' returned {} results",
        results.len()
    );
    results
}
