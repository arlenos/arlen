//! Tauri commands that bridge the desktop-shell frontend to
//! `arlen-modulesd`.
//!
//! All real module work happens in the daemon. This file is a thin
//! translation layer: take a Tauri command, build the corresponding
//! `modulesd_proto::Request`, send it over the socket via
//! `ModulesdClient::call`, and return the deserialised payload that
//! the frontend can use directly.
//!
//! Two reasons for putting this between the frontend and the client:
//!   1. The frontend should never see protocol envelopes. It calls
//!      `mint_iframe(...)` and gets a typed object back, not a
//!      `Response::IframeIssued`.
//!   2. We can centralise the "daemon not connected" fallback in one
//!      place and surface a uniform `ClientError` to the frontend.

use std::sync::Arc;

use modulesd_proto::{ErrorCode, HostCall, HostReply, ModuleSummary, Request, Response};
use serde::Serialize;

use arlen_desktop_shell_core::modulesd_client::ModulesdClient;

/// Frontend-facing module summary. Mirrors `modulesd_proto::ModuleSummary`
/// but with camelCase field names so the JSON sits naturally in
/// TypeScript without an extra mapping layer.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiModule {
    pub id: String,
    pub name: String,
    pub version: String,
    pub tier: String,
    pub enabled: bool,
    pub failed: bool,
    pub priority: u32,
    pub extension_points: Vec<String>,
}

impl From<ModuleSummary> for UiModule {
    fn from(m: ModuleSummary) -> Self {
        Self {
            id: m.id,
            name: m.name,
            version: m.version,
            tier: match m.tier {
                modulesd_proto::ModuleTier::Wasm => "wasm".into(),
                modulesd_proto::ModuleTier::Iframe => "iframe".into(),
            },
            enabled: m.enabled,
            failed: m.failed,
            priority: m.priority,
            extension_points: m.extension_points,
        }
    }
}

/// Frontend-facing iframe issuance.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiIframe {
    pub url: String,
    pub nonce: String,
}

/// Frontend-facing host-call reply.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum UiHostReply {
    GraphResult { rows: String },
    NetworkBody { status: u16, body_b64: String },
    Acked,
    Error { code: String, message: String },
}

impl From<HostReply> for UiHostReply {
    fn from(r: HostReply) -> Self {
        match r {
            HostReply::GraphResult { rows } => Self::GraphResult { rows },
            HostReply::NetworkBody { status, body_b64 } => Self::NetworkBody {
                status,
                body_b64,
            },
            HostReply::Acked => Self::Acked,
            HostReply::Error { code, message } => Self::Error {
                code: format!("{code:?}").to_lowercase(),
                message,
            },
        }
    }
}

/// Helper: send a request and unwrap the typed response, surfacing
/// daemon errors as Tauri-friendly strings.
async fn call(
    client: &Arc<ModulesdClient>,
    req: Request,
) -> Result<Response, String> {
    client.call(req).await.map_err(|e| {
        // The transport, not the daemon's verdict: a socket that is not there or
        // a reply that will not parse. `internal` is the honest token for it, and
        // the system's own words go to the log.
        log::warn!("modulesd call failed: {e}");
        refusal(ErrorCode::Internal)
    })
}

/// The token a refusal travels to the window as.
///
/// `Response::Error` has carried a machine-readable `code` beside its `message`
/// all along, and every shim below threw the code away and returned the message.
/// One of them reaches a translated sentence - the module host's "did not mount"
/// tooltip filled its `{$why}` with it - so a German reader met "module
/// com.example.x not found" inside a German clause. The code was already there;
/// nothing was reading it.
///
/// Kebab-case, matching the refusal tokens the other app surfaces use
/// (`not-permitted`, `file-changed-on-disk`), NOT the `snake_case` the proto
/// serialises: this word is for a window, and one shape across the windows is
/// worth more here than agreement with a wire nobody reading it will see. The
/// code that goes back to a MODULE keeps the wire form, below.
fn refusal(code: ErrorCode) -> String {
    match code {
        ErrorCode::NotFound => "not-found",
        ErrorCode::PermissionDenied => "permission-denied",
        ErrorCode::ModuleFailed => "module-failed",
        ErrorCode::Timeout => "timeout",
        ErrorCode::InvalidRequest => "invalid-request",
        ErrorCode::Internal => "internal",
    }
    .to_string()
}

/// A reply that is neither the expected one nor a named error.
///
/// Its debug form names internal types, so it goes to the log and the window is
/// told the one true thing: something went wrong inside.
fn unexpected(what: &str, other: &Response) -> String {
    log::warn!("{what}: unexpected reply {other:?}");
    refusal(ErrorCode::Internal)
}

#[cfg(test)]
mod refusal_tests {
    use super::*;

    #[test]
    fn every_code_has_a_word_and_none_of_them_is_a_sentence() {
        // The window looks each of these up in its catalogue. A token that read
        // as a sentence would be shown as one, which is the whole defect this
        // replaced.
        for code in [
            ErrorCode::NotFound,
            ErrorCode::PermissionDenied,
            ErrorCode::ModuleFailed,
            ErrorCode::Timeout,
            ErrorCode::InvalidRequest,
            ErrorCode::Internal,
        ] {
            let t = refusal(code);
            assert!(!t.contains(' '), "token {t} reads as a sentence");
            assert!(
                t.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "token {t} is not kebab-case"
            );
        }
    }

    #[test]
    fn the_code_a_module_receives_is_the_wire_spelling() {
        // `format!("{:?}").to_lowercase()` gave `notfound`; the proto says
        // `not_found`. A module matching the documented code never matched.
        assert_eq!(
            serde_json::to_value(ErrorCode::NotFound).unwrap().as_str(),
            Some("not_found")
        );
        assert_eq!(
            serde_json::to_value(ErrorCode::PermissionDenied).unwrap().as_str(),
            Some("permission_denied")
        );
    }
}

/// List every module the daemon knows about. The Phase-7-style
/// `list_modules` command kept by `modules.rs` is removed in M4; this
/// `modulesd_list_modules` is the canonical shell-facing entry point
/// from now on.
#[tauri::command]
pub async fn modulesd_list_modules(
    client: tauri::State<'_, Arc<ModulesdClient>>,
) -> Result<Vec<UiModule>, String> {
    let resp = call(
        client.inner(),
        Request::ListModules { id: String::new() },
    )
    .await?;
    match resp {
        Response::ModuleList { modules, .. } => {
            Ok(modules.into_iter().map(UiModule::from).collect())
        }
        Response::Error { code, message, .. } => {
            log::warn!("modulesd refused: {code:?}: {message}");
            Err(refusal(code))
        }
        ref other => Err(unexpected("modulesd", other)),
    }
}

/// Mint a Tier 2 iframe URL for a module. Returns `(url, nonce)`.
#[tauri::command]
pub async fn mint_iframe(
    module_id: String,
    slot: String,
    client: tauri::State<'_, Arc<ModulesdClient>>,
) -> Result<UiIframe, String> {
    let resp = call(
        client.inner(),
        Request::IframeMint {
            id: String::new(),
            module_id,
            slot,
        },
    )
    .await?;
    match resp {
        Response::IframeIssued { url, nonce, .. } => Ok(UiIframe { url, nonce }),
        Response::Error { code, message, .. } => {
            log::warn!("modulesd refused: {code:?}: {message}");
            Err(refusal(code))
        }
        ref other => Err(unexpected("modulesd", other)),
    }
}

/// Forward a postMessage `host.call` from the iframe to modulesd for
/// capability-checked execution. Returns the typed reply for the
/// shell to relay back to the iframe.
#[tauri::command]
pub async fn module_host_call(
    nonce: String,
    call_payload: HostCall,
    client: tauri::State<'_, Arc<ModulesdClient>>,
) -> Result<UiHostReply, String> {
    let resp = call(
        client.inner(),
        Request::HostCall {
            id: String::new(),
            nonce,
            call: call_payload,
        },
    )
    .await?;
    match resp {
        Response::HostReply { reply, .. } => Ok(reply.into()),
        Response::Error { message, code, .. } => Ok(UiHostReply::Error {
            // The WIRE spelling, for a module author reading the proto. It used
            // to be `format!("{code:?}").to_lowercase()`, which turns `NotFound`
            // into `notfound` while the proto's own `rename_all = "snake_case"`
            // makes it `not_found` - so a module matching the documented code
            // never matched. Serialised through serde so the two cannot drift.
            code: serde_json::to_value(code)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "internal".to_string()),
            message,
        }),
        ref other => Err(unexpected("module host call", other)),
    }
}

/// Toggle a module's enabled state. Daemon revokes any live nonces
/// belonging to the module on disable, so the shell should also
/// remove the iframe element after this call returns.
#[tauri::command]
pub async fn modulesd_set_enabled(
    module_id: String,
    enabled: bool,
    client: tauri::State<'_, Arc<ModulesdClient>>,
) -> Result<(), String> {
    let resp = call(
        client.inner(),
        Request::SetEnabled {
            id: String::new(),
            module_id,
            enabled,
        },
    )
    .await?;
    match resp {
        Response::Acked { .. } => Ok(()),
        Response::Error { code, message, .. } => {
            log::warn!("modulesd refused: {code:?}: {message}");
            Err(refusal(code))
        }
        ref other => Err(unexpected("modulesd", other)),
    }
}

/// Manual retry for a permanently-failed module.
#[tauri::command]
pub async fn retry_module(
    module_id: String,
    client: tauri::State<'_, Arc<ModulesdClient>>,
) -> Result<(), String> {
    let resp = call(
        client.inner(),
        Request::Retry {
            id: String::new(),
            module_id,
        },
    )
    .await?;
    match resp {
        Response::Acked { .. } => Ok(()),
        Response::Error { code, message, .. } => {
            log::warn!("modulesd refused: {code:?}: {message}");
            Err(refusal(code))
        }
        ref other => Err(unexpected("modulesd", other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modulesd_proto::{ErrorCode, ModuleTier};

    #[test]
    fn ui_module_translates_tier_enum() {
        let s = ModuleSummary {
            id: "x".into(),
            name: "X".into(),
            version: "1.0".into(),
            tier: ModuleTier::Iframe,
            enabled: true,
            last_error: None,
            failed: false,
            priority: 100,
            extension_points: vec!["topbar".into()],
            granted: Vec::new(),
        };
        let ui: UiModule = s.into();
        assert_eq!(ui.tier, "iframe");
    }

    #[test]
    fn ui_host_reply_lowercases_error_code() {
        let r = HostReply::Error {
            code: ErrorCode::PermissionDenied,
            message: "no".into(),
        };
        let ui: UiHostReply = r.into();
        match ui {
            UiHostReply::Error { code, .. } => assert_eq!(code, "permissiondenied"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn ui_host_reply_handles_acked() {
        let ui: UiHostReply = HostReply::Acked.into();
        assert!(matches!(ui, UiHostReply::Acked));
    }
}
