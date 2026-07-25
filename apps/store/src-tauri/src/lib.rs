//! Arlen store app backend.
//!
//! Thin Tauri proxy over the `org.arlen.Store1` socket (store-app.md section
//! 9.4). Each command forwards one request to the running `store-backend` and
//! hands back its `Response` payload verbatim (`AppCard`, `Variant`,
//! `TrustSignals`, ...). The view model - the plain-language capability lines,
//! the tier and facet flags, the least-privilege weight - is derived in the
//! i18n'd frontend, not here: the backend must not emit user-facing copy or it
//! would ship one language. "arlen-ui designs against this surface."

use arlen_store_backend::{
    request_default, AppCard, CapabilityFacet, ComponentId, Request, Response, SourceLayer,
    TrustSignals, Variant,
};
use serde::Serialize;

/// Forward one request to the store backend, mapping a transport failure to a
/// string the frontend surfaces.
async fn ask(req: Request) -> Result<Response, String> {
    request_default(&req).await.map_err(|e| e.to_string())
}

/// Full-text search over the merged catalog, narrowed by capability facets.
/// Returns the backend `AppCard`s; the frontend maps them to its view model.
#[tauri::command]
async fn store_search(query: String, facets: Vec<CapabilityFacet>) -> Result<Vec<AppCard>, String> {
    match ask(Request::Search { query, facets }).await? {
        Response::Cards(cards) => Ok(cards),
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// The full merged card for an id, or `None` when the id is unknown (a clean
/// not-found for the app page, not an error).
#[tauri::command]
async fn store_app_detail(id: String) -> Result<Option<AppCard>, String> {
    match ask(Request::AppDetail { id: ComponentId(id) }).await? {
        Response::Card(card) => Ok(card),
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

/// A validated install handoff: the id + resolved variant the caller drives
/// through the consent friction-ladder. The backend does not install here.
#[derive(Serialize)]
struct InstallHandoff {
    id: String,
    variant: SourceLayer,
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
        Response::InstallResolved { id, variant } => {
            Ok(InstallHandoff { id: id.0, variant })
        }
        Response::Error(e) => Err(e),
        other => Err(format!("unexpected store response: {other:?}")),
    }
}

/// The local observed-vs-declared summary for an id (an audit-ledger read), or
/// `None` when nothing has been recorded yet.
#[tauri::command]
async fn store_observed_vs_declared(id: String) -> Result<Option<String>, String> {
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
            store_search,
            store_app_detail,
            store_trust_signals,
            store_variants,
            store_install,
            store_observed_vs_declared,
            frontend_log,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
