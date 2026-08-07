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


mod url;

use std::collections::BTreeSet;

use arlen_store_backend::{
    request_default, store_card, store_cards, CapabilityFacet, ComponentId, ObservedStatus,
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
        Response::Cards(cards) => Ok(store_cards(&cards, &installed_ids().await)),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// The flattened card for an id, or `None` when the id is unknown (a clean
/// not-found for the app page, not an error).
#[tauri::command]
async fn store_app_detail(id: String) -> Result<Option<StoreCard>, String> {
    match ask(Request::AppDetail { id: ComponentId(id) }).await? {
        Response::Card(Some(card)) => Ok(Some(store_card(&card, &installed_ids().await))),
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
            store_observed_vs_declared,
            store_outdated,
            store_skip_update,
            frontend_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
