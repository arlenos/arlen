//! Arlen store app backend.
//!
//! Thin Tauri proxy over the `org.arlen.Store1` socket (store-app.md section
//! 9.4). Each command forwards one request to the running `store-backend`.
//!
//! The browse commands flatten the merged `AppCard` into the `StoreCard` the app
//! renders (SC-2), fusing in the installed set from installd. The flattening
//! itself is the tested `arlen_store_backend::view` logic, so the derivation of
//! the facets, the least-privilege weight and the tier lives in one place and is
//! covered by CI. What stays out of it is COPY: the app is translated (`st.*`,
//! en + de), so the card carries capability *identifiers* and the frontend
//! renders each into its own language. A Rust backend writing "Cannot reach the
//! network" would ship one language.


mod icon_scheme;
mod url;

use std::collections::BTreeSet;

use arlen_store_backend::{
    request_default, store_card, store_cards, CapabilityFacet, CatalogSources, Collection,
    ComponentId, ObservedStatus,
    PendingUpdate, Request, Response, SortOrder, SourceLayer, StoreCard, TrustSignals, Variant,
};
use serde::Serialize;

/// installd's well-known name, object and interface (all three coincide).
const INSTALLD: &str = "org.arlen.InstallDaemon1";
/// installd's object path.
const INSTALLD_PATH: &str = "/org/arlen/InstallDaemon1";

/// Forward one request to the store backend, mapping a transport failure to a
/// string the frontend surfaces.
async fn ask(req: Request) -> Result<Response, String> {
    request_default(&req).await.map_err(|e| e.to_string())
}

/// The component-ids installd reports installed. Degrades to an empty set when
/// the daemon is unreachable: a card then renders as not-installed, which is the
/// honest default (never claim something is installed on a failed read).
async fn installed_ids() -> BTreeSet<String> {
    match fetch_installed().await {
        Ok(apps) => apps.into_iter().map(|(id, _, _, _)| id).collect(),
        Err(e) => {
            log::warn!("installed_ids: install daemon unavailable: {e}");
            BTreeSet::new()
        }
    }
}

/// Call `org.arlen.InstallDaemon1.ListInstalled`, returning its
/// `(id, name, version, source)` tuples.
async fn fetch_installed() -> Result<Vec<(String, String, String, String)>, zbus::Error> {
    let conn = zbus::Connection::session().await?;
    let proxy = zbus::Proxy::new(&conn, INSTALLD, INSTALLD_PATH, INSTALLD).await?;
    proxy.call("ListInstalled", &()).await
}

/// Full-text search over the merged catalog, narrowed by capability facets.
/// Returns the flattened cards the browse grid renders.
///
/// `sort` is optional and defaults to catalog order. Asking for least-privilege
/// puts the app that requests the least first, which is what declaring
/// capabilities rather than inferring them makes possible.
#[tauri::command]
async fn store_search(
    query: String,
    facets: Vec<CapabilityFacet>,
    sort: Option<SortOrder>,
) -> Result<Vec<StoreCard>, String> {
    match ask(Request::Search {
        query,
        facets,
        sort: sort.unwrap_or_default(),
    })
    .await?
    {
        Response::Cards(cards) => Ok(icon_scheme::repaint_all(
            store_cards(&cards, &installed_ids().await),
        )),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// The flattened card for an id, or `None` when the id is unknown (a clean
/// not-found for the app page, not an error).
#[tauri::command]
async fn store_app_detail(id: String) -> Result<Option<StoreCard>, String> {
    match ask(Request::AppDetail { id: ComponentId(id) }).await? {
        Response::Card(Some(card)) => Ok(Some(icon_scheme::repaint(store_card(
            &card,
            &installed_ids().await,
        )))),
        Response::Card(None) => Ok(None),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// The per-variant trust signals for an id (variant layer + its signals).
#[tauri::command]
async fn store_trust_signals(id: String) -> Result<Vec<(SourceLayer, TrustSignals)>, String> {
    match ask(Request::TrustSignals { id: ComponentId(id) }).await? {
        Response::Trust(t) => Ok(t),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// The install variants for an id, highest-precedence first.
#[tauri::command]
async fn store_variants(id: String) -> Result<Vec<Variant>, String> {
    match ask(Request::Variants { id: ComponentId(id) }).await? {
        Response::Variants(v) => Ok(v),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// A validated install handoff: the id, the resolved variant, and what that
/// variant's installer needs to be told. The caller drives it through the consent
/// friction-ladder; the backend does not install here.
#[derive(Serialize)]
struct InstallHandoff {
    id: String,
    variant: SourceLayer,
    /// The Debian package name, Flatpak ref or forage recipe id for this variant.
    ///
    /// `None` when the catalog knows none, which the frontend must surface as
    /// "cannot install this variant" - deriving one from the component id is
    /// right for Flathub by convention and wrong for apt, where the package name
    /// is a separate field precisely because it differs.
    install_handle: Option<String>,
}

/// Validate + resolve an install target. When `variant` is absent (the primary
/// Install button), the card's default (highest-precedence) variant is resolved
/// first, so the app never has to know the precedence order.
#[tauri::command]
async fn store_install(
    id: String,
    variant: Option<SourceLayer>,
) -> Result<InstallHandoff, String> {
    let variant = match variant {
        Some(v) => v,
        None => match ask(Request::AppDetail { id: ComponentId(id.clone()) }).await? {
            Response::Card(Some(card)) => card
                .variants
                .get(card.default_variant)
                .map(|v| v.layer)
                .ok_or_else(|| "no installable variant".to_string())?,
            Response::Card(None) => return Err(format!("unknown app: {id}")),
            Response::Error(e) => return Err(e),
            other => return Err(format!("unexpected store response: {other:?}")),
        },
    };
    match ask(Request::Install { id: ComponentId(id), variant }).await? {
        Response::InstallResolved { id, variant, install_handle } => {
            Ok(InstallHandoff { id: id.0, variant, install_handle })
        }
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// Which layer an app was installed from, as installd records it.
///
/// ASKED rather than taken from the caller: installd owns what is installed, and a
/// caller supplying the layer could aim a Flatpak removal at a lunpkg app and get a
/// refusal it could not explain. An id it does not know is refused here, so
/// "you never installed that" does not arrive as a D-Bus error about a missing app.
async fn installed_source(id: &str) -> Result<String, String> {
    let installed = fetch_installed().await.map_err(|e| e.to_string())?;
    installed
        .iter()
        .find(|(app_id, _, _, _)| app_id == id)
        .map(|(_, _, _, source)| source.clone())
        .ok_or_else(|| format!("{id} is not installed"))
}

/// The installd method that removes an app installed from `source`.
///
/// Both tokens are named and anything else is refused. The strings are installd's -
/// `flatpak::list_installed_flatpaks` pushes "flatpak", `install::list_installed`
/// pushes "lunpkg" - and nothing binds them to this file. A default arm would send
/// an unrecognised source down the lunpkg path, where it would fail somewhere
/// deeper, or worse succeed at removing the wrong thing. Pure, so the routing is
/// tested without a bus.
fn removal_method(source: &str) -> Option<&'static str> {
    match source {
        "flatpak" => Some("UninstallFlatpak"),
        "lunpkg" => Some("Uninstall"),
        _ => None,
    }
}

/// Uninstall an installed app.
///
/// WHICH METHOD depends on where the app came from, and the answer is read from
/// installd rather than passed in: it owns the record of what is installed, and a
/// caller supplying the layer could aim a Flatpak removal at a lunpkg app and get
/// a refusal it could not explain. An id installd does not know is refused here
/// rather than sent, so "you never installed that" does not arrive as a D-Bus
/// error about a missing app.
///
/// Answers the job id. The removal itself happens on installd's queue and its
/// outcome arrives on `JobCompleted`; installd refuses outright anything that is
/// part of the desktop, so this cannot be used to remove Settings or the shell.
#[tauri::command]
async fn store_uninstall(id: String) -> Result<String, String> {
    let source = installed_source(&id).await?;
    let method = removal_method(&source).ok_or_else(|| {
        format!("{id} is recorded as installed from {source}, which this build has no way to remove")
    })?;
    let conn = zbus::Connection::session()
        .await
        .map_err(|e| e.to_string())?;
    let proxy = zbus::Proxy::new(&conn, INSTALLD, INSTALLD_PATH, INSTALLD)
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .call(method, &(id,))
        .await
        .map_err(|e| e.to_string())
}

/// Apply the update pending for one app.
///
/// ONLY THE FLATPAK LAYER, and the refusal for the rest is the honest half. A
/// lunpkg update is `installd.Update(path)`, which takes a LOCAL package file, and
/// nothing on this machine fetches one - so a Debian or forage app has no update
/// path to call yet and this says which layer it is rather than failing somewhere
/// deeper with a message about a missing file.
///
/// installd's job refuses a version asking for more than the installed one and
/// emits `ConsentRequired`, so accepting a widening is not something this button
/// can do by itself.
#[tauri::command]
async fn store_update(id: String) -> Result<String, String> {
    let source = installed_source(&id).await?;
    // Named rather than defaulted, for the reason `removal_method` gives: an
    // unrecognised source is not a lunpkg.
    match source.as_str() {
        "flatpak" => {}
        "lunpkg" => {
            return Err(format!(
                "{id} was installed as a package file, and updating that layer needs \
                 one nothing on this machine fetches yet"
            ))
        }
        other => {
            return Err(format!(
                "{id} is recorded as installed from {other}, which this build has no \
                 way to update"
            ))
        }
    }
    let conn = zbus::Connection::session()
        .await
        .map_err(|e| e.to_string())?;
    let proxy = zbus::Proxy::new(&conn, INSTALLD, INSTALLD_PATH, INSTALLD)
        .await
        .map_err(|e| e.to_string())?;
    proxy
        .call("UpdateFlatpak", &(id,))
        .await
        .map_err(|e| e.to_string())
}

/// Apply every routine update: the ones asking for nothing new.
///
/// The caller has already filtered to those, and this does not re-derive it -
/// installd re-checks each one anyway and refuses a widening, so the worst a
/// wrong list can do is turn a row into a refusal rather than apply something
/// unexamined.
///
/// Answers the job ids that were enqueued. A failure part-way is returned with
/// whatever was already started rather than swallowed: some updates having run is
/// the true state, and reporting it as total failure would send somebody looking
/// for changes that did happen.
#[tauri::command]
async fn store_update_all_routine(ids: Vec<String>) -> Result<Vec<String>, String> {
    let mut jobs = Vec::new();
    for id in ids {
        match store_update(id.clone()).await {
            Ok(job) => jobs.push(job),
            Err(e) if jobs.is_empty() => return Err(e),
            Err(e) => {
                log::warn!("update {id} refused after {} started: {e}", jobs.len());
                break;
            }
        }
    }
    Ok(jobs)
}

/// The installed apps whose own source now offers a different version.
///
/// A local read of the cached catalog against the install lock, so opening a
/// page that shows updates does not become a request to every source the user
/// has. The backend compares only within the layer an app was installed from,
/// and reports that the versions differ rather than that one is newer: ordering
/// distro version strings is per-layer and getting it wrong either hides updates
/// or offers downgrades. Both versions come back so the app can show them.
///
/// Returned unflattened, unlike the browse commands: a [`PendingUpdate`] is
/// already the flat row, and it carries capability identifiers rather than copy
/// for the same reason the cards do - the app is translated and the backend is
/// not.
#[tauri::command]
async fn store_outdated() -> Result<Vec<PendingUpdate>, String> {
    match ask(Request::Outdated).await? {
        Response::Updates(u) => Ok(u),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// Stop offering the update currently pending for this app.
///
/// The backend resolves which version that is, so a caller cannot park an id on
/// a version that was never on offer. Returns the remaining set, which is what
/// the list should show: the frontend drops the row optimistically and this is
/// the system's own answer to reconcile against.
#[tauri::command]
async fn store_skip_update(id: String) -> Result<Vec<PendingUpdate>, String> {
    match ask(Request::SkipUpdate { id: ComponentId(id) }).await? {
        Response::Updates(u) => Ok(u),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// What can honestly be said about this app's observed-vs-declared standing
/// (store-app.md section 8.2). Structured, not prose: the app renders it in its
/// own language, and it distinguishes "no feed yet" from "nothing observed" so
/// the panel never reads as a clean bill of health the system cannot give.
#[tauri::command]
async fn store_observed_vs_declared(id: String) -> Result<ObservedStatus, String> {
    match ask(Request::ObservedVsDeclared { id: ComponentId(id) }).await? {
        Response::Observed(o) => Ok(o),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// The editorial collections the landing view shows before anyone types
/// (store-app.md section 8.7), already narrowed to apps this machine's catalog
/// carries.
///
/// The narrowing is the reason this is an op rather than a constant in the page.
/// A hardcoded collection names ids that exist in a fixture, and against a live
/// catalog every one of them resolves to nothing, so the store's landing view
/// renders empty over a catalog full of apps. Here the backend intersects the
/// curated list with what it actually has and drops a collection left with
/// nothing, so a heading is never shown over empty space.
///
/// Titles arrive per locale from the curator's own file rather than as
/// identifiers, which is the one deliberate exception to this app's
/// no-copy-from-Rust rule: a collection's name belongs to whoever picked it, and
/// a curator who needs an app release to add one is not curating.
#[tauri::command]
async fn store_collections() -> Result<Vec<Collection>, String> {
    match ask(Request::Collections).await? {
        Response::Collections(c) => Ok(c),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// Which app-metadata sources this machine actually has.
///
/// An empty grid has two causes that look identical: nothing matched, or there
/// is no catalog on this machine at all. The second is the state of a fresh
/// image - it ships no MetaInfo, no Flatpak remote and no DEP-11 - and drawing
/// both as blank space tells somebody their store is broken when it is only
/// unfurnished. Counts rather than flags, because one document and eight hundred
/// are both "present" and only one of them is a furnished store.
#[tauri::command]
async fn store_sources() -> Result<CatalogSources, String> {
    match ask(Request::Sources).await? {
        Response::Sources(s) => Ok(s),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// Route a frontend log line into the backend's stdout (Tim cannot open the
/// webview devtools; this is the diagnostic channel).
#[tauri::command]
fn frontend_log(level: String, message: String) {
    match level.as_str() {
        "error" => log::error!("[frontend] {message}"),
        "warn" => log::warn!("[frontend] {message}"),
        _ => log::info!("[frontend] {message}"),
    }
}

/// Tauri application entry point invoked from `main.rs`.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            url::open_url,
            store_search,
            store_app_detail,
            store_trust_signals,
            store_variants,
            store_install,
            store_uninstall,
            store_update,
            store_update_all_routine,
            store_observed_vs_declared,
            store_outdated,
            store_skip_update,
            store_collections,
            store_sources,
            frontend_log,
        ])
        // The catalogue's icons are files on this machine and a webview cannot
        // open a path, so they are served over a scheme of their own. See
        // `icon_scheme` for what it will and will not serve.
        // The catalogue's icons are files this window may not read - its profile
        // has no filesystem grant and argues for that - so the handler forwards
        // an id to the backend and serves what comes back. Asynchronous because
        // that forward is a socket round trip.
        .register_asynchronous_uri_scheme_protocol("icon", |_app, request, responder| {
            tauri::async_runtime::spawn(async move {
                responder.respond(icon_scheme::handle(request).await);
            });
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The routing between installd's two source tokens and its two removal
    /// methods, and the refusal for anything else.
    ///
    /// Worth a test rather than a reading, because the tokens are agreed across a
    /// process boundary with nothing binding them: installd emits them and this
    /// file compares them, so a rename there is silent here. The refusal is what
    /// turns that silence into a sentence.
    #[test]
    fn a_source_this_build_does_not_know_is_refused_rather_than_routed() {
        assert_eq!(removal_method("flatpak"), Some("UninstallFlatpak"));
        assert_eq!(removal_method("lunpkg"), Some("Uninstall"));

        // The shapes a rename or a new layer would arrive as. Each has to answer
        // None: routing an unknown source to the lunpkg method would aim a removal
        // at the wrong subsystem.
        for unknown in ["Flatpak", "flatpak ", "apt", "forage", "snap", ""] {
            assert_eq!(removal_method(unknown), None, "{unknown} must not route");
        }
    }
}
