/// Tauri commands that bridge the desktop-shell frontend to
/// `lunaris-modulesd`.
///
/// All real module work happens in the daemon. This file is a thin
/// translation layer: take a Tauri command, build the corresponding
/// `modulesd_proto::Request`, send it over the socket via
/// `ModulesdClient::call`, and return the deserialised payload that
/// the frontend can use directly.
///
/// Two reasons for putting this between the frontend and the client:
///   1. The frontend should never see protocol envelopes. It calls
///      `mint_iframe(...)` and gets a typed object back, not a
///      `Response::IframeIssued`.
///   2. We can centralise the "daemon not connected" fallback in one
///      place and surface a uniform `ClientError` to the frontend.

use std::sync::Arc;

use modulesd_proto::{HostCall, HostReply, ModuleSummary, Request, Response};
use serde::Serialize;

use crate::modulesd_client::ModulesdClient;

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
    client.call(req).await.map_err(|e| e.to_string())
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
        Response::Error { message, .. } => Err(message),
        other => Err(format!("unexpected reply: {other:?}")),
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
        Response::Error { message, .. } => Err(message),
        other => Err(format!("unexpected reply: {other:?}")),
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
            code: format!("{code:?}").to_lowercase(),
            message,
        }),
        other => Err(format!("unexpected reply: {other:?}")),
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
        Response::Error { message, .. } => Err(message),
        other => Err(format!("unexpected reply: {other:?}")),
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
        Response::Error { message, .. } => Err(message),
        other => Err(format!("unexpected reply: {other:?}")),
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
            failed: false,
            priority: 100,
            extension_points: vec!["topbar".into()],
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
