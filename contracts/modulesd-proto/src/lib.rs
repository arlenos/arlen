/// Wire protocol between desktop-shell and `arlen-modulesd`.
///
/// JSON over a Unix socket. Each frame is `[u32 BE length][JSON body]`.
/// JSON instead of protobuf because the consumers are predominantly
/// TypeScript (desktop-shell, Settings) and the per-frame cost of a
/// search round-trip is negligible compared to a Wasmtime call.
///
/// Three top-level message kinds:
///   * `Request`  — shell → daemon, expects exactly one `Response`.
///   * `Response` — daemon → shell, replies to a Request by id.
///   * `Event`    — daemon → shell, unsolicited (lifecycle, broadcasts).

use serde::{Deserialize, Serialize};

/// Plugin priority on the wire; matches `WaypointerPlugin::priority`
/// in `module-sdk` (lower = higher priority, system 0-9, first-party
/// 10-99, third-party 100+).
pub type PluginPriority = u32;

/// Shell → daemon. Every Request carries an `id` that the matching
/// Response echoes back so concurrent calls can be paired.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Initial handshake. Client identifies itself; daemon replies
    /// with `Response::Hello` carrying its protocol version.
    Hello {
        id: String,
        client: String,
        version: String,
    },

    /// Enumerate every module known to the daemon. Used by Settings.
    ListModules { id: String },

    /// Run a Tier 1 `waypointer.search` call against a single module.
    WaypointerSearch {
        id: String,
        module_id: String,
        query: String,
    },

    /// Run a Tier 1 `waypointer.search` call against every enabled
    /// module that matches the query (no prefix filtering yet; daemon
    /// does it).
    WaypointerSearchAll { id: String, query: String },

    /// Execute a search result (delegates to the module's `execute`).
    WaypointerExecute {
        id: String,
        module_id: String,
        result: SearchResult,
    },

    /// Request a Tier 2 iframe URL for the given module. Daemon mints
    /// a nonce and returns a `module://` URL ready to drop into a
    /// `src` attribute.
    IframeMint {
        id: String,
        module_id: String,
        slot: String,
    },

    /// Proxy a postMessage host call from a Tier 2 iframe to the
    /// daemon. The call is gated against the iframe's
    /// `CapabilityContext` before being executed.
    HostCall {
        id: String,
        nonce: String,
        call: HostCall,
    },

    /// Subscribe to lifecycle events. Daemon broadcasts every event
    /// matching one of the requested kinds to this connection.
    Subscribe {
        id: String,
        kinds: Vec<EventKind>,
    },

    /// Enable or disable a module. Persisted to
    /// `~/.config/arlen/modules.toml`.
    SetEnabled {
        id: String,
        module_id: String,
        enabled: bool,
    },

    /// Manual retry on a failed module (Settings → Modules → Retry).
    Retry { id: String, module_id: String },

    /// Resolve a Tier 2 iframe nonce to its bound module and the CSP
    /// header that should be served. Used by the desktop-shell
    /// `module://` scheme handler on every asset fetch.
    IframeLookup { id: String, nonce: String },
}

/// Daemon → shell. Replies to Requests; correlated by `id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Hello {
        id: String,
        version: String,
    },
    ModuleList {
        id: String,
        modules: Vec<ModuleSummary>,
    },
    WaypointerResults {
        id: String,
        module_id: String,
        results: Vec<SearchResult>,
    },
    WaypointerAggregate {
        id: String,
        results: Vec<SearchResult>,
    },
    Executed {
        id: String,
    },
    IframeIssued {
        id: String,
        url: String,
        nonce: String,
    },
    HostReply {
        id: String,
        reply: HostReply,
    },
    Subscribed {
        id: String,
    },
    Acked {
        id: String,
    },
    /// Generic typed error reply.
    Error {
        id: String,
        code: ErrorCode,
        message: String,
    },

    /// Reply to `IframeLookup`. `module_id` is the bound module
    /// identifier; `csp` is the per-module CSP header value the
    /// scheme handler should attach to every response.
    IframeMeta {
        id: String,
        module_id: String,
        root_path: String,
        csp: String,
    },
}

/// Daemon → shell. Unsolicited broadcasts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    ModuleLoaded { module_id: String },
    ModuleUnloaded { module_id: String },
    ModuleCrashed {
        module_id: String,
        crashes: u32,
        next_action: String,
    },
    ModuleFailed { module_id: String },
    ModuleEnabled { module_id: String },
    ModuleDisabled { module_id: String },
}

/// Tag used to filter what events a subscriber wants.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    ModuleLifecycle,
    ModuleEnabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSummary {
    pub id: String,
    pub name: String,
    pub version: String,
    pub tier: ModuleTier,
    pub enabled: bool,
    pub failed: bool,
    /// Lower means higher priority; matches `WaypointerPlugin::priority`.
    pub priority: PluginPriority,
    pub extension_points: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModuleTier {
    Wasm,
    Iframe,
}

/// Subset of `module-sdk::SearchResult`. We re-shape it on the wire so
/// the JSON stays stable even if the SDK type evolves; the conversion
/// is in `manager`. Mirror of the Tier 1 WIT `search-result`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub relevance: f32,
    pub action: SearchAction,
    pub plugin_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchAction {
    Copy { text: String },
    OpenUrl { url: String },
    OpenPath { path: String },
    Execute { command: String },
    Custom { handler: String, data: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostCall {
    GraphQuery { cypher: String },
    GraphWrite { cypher: String },
    NetworkFetch { url: String, headers: Vec<(String, String)> },
    /// HTTP POST host call for Tier 2 iframes. `body_b64` is the
    /// base64-encoded request body so the JSON wire format stays
    /// safe for arbitrary bytes. Header semantics + capability
    /// gating match `NetworkFetch`; the SDK `host::network::post`
    /// helper runs the same HTTPS-only + SSRF + per-hop redirect
    /// re-validation pipeline.
    NetworkPost {
        url: String,
        body_b64: String,
        headers: Vec<(String, String)>,
    },
    EventEmit { event_type: String, payload_b64: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostReply {
    GraphResult { rows: String },
    NetworkBody { status: u16, body_b64: String },
    Acked,
    Error { code: ErrorCode, message: String },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    PermissionDenied,
    ModuleFailed,
    Timeout,
    InvalidRequest,
    Internal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trip_via_json() {
        let req = Request::WaypointerSearch {
            id: "r1".into(),
            module_id: "com.example.test".into(),
            query: "hello".into(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: Request = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, Request::WaypointerSearch { .. }));
    }

    #[test]
    fn response_with_results_round_trip() {
        let r = Response::WaypointerResults {
            id: "r1".into(),
            module_id: "com.example.test".into(),
            results: vec![SearchResult {
                id: "x".into(),
                title: "Hi".into(),
                description: None,
                icon: None,
                relevance: 1.0,
                action: SearchAction::Copy { text: "hi".into() },
                plugin_id: "com.example.test".into(),
            }],
        };
        let json = serde_json::to_string(&r).unwrap();
        let _: Response = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn event_serialisation_uses_snake_case_tag() {
        let ev = Event::ModuleCrashed {
            module_id: "x".into(),
            crashes: 2,
            next_action: "delayed:5s".into(),
        };
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains("\"type\":\"module_crashed\""));
    }

    #[test]
    fn host_call_variants_round_trip() {
        let calls = vec![
            HostCall::GraphQuery { cypher: "MATCH (n) RETURN n".into() },
            HostCall::NetworkFetch {
                url: "https://api.example.com".into(),
                headers: vec![("X-Foo".into(), "bar".into())],
            },
            HostCall::EventEmit {
                event_type: "module.refreshed".into(),
                payload_b64: "eyJrIjogInYifQ==".into(),
            },
        ];
        for call in calls {
            let json = serde_json::to_string(&call).unwrap();
            let _: HostCall = serde_json::from_str(&json).unwrap();
        }
    }

    /// The `Request` `type` tag is the wire contract between the shell and the
    /// daemon; a renamed variant must fail a test, not silently break IPC.
    #[test]
    fn request_tags_are_stable() {
        let cases: Vec<(Request, &str)> = vec![
            (Request::Hello { id: "i".into(), client: "c".into(), version: "1".into() }, "hello"),
            (Request::ListModules { id: "i".into() }, "list_modules"),
            (
                Request::WaypointerSearch { id: "i".into(), module_id: "m".into(), query: "q".into() },
                "waypointer_search",
            ),
            (Request::WaypointerSearchAll { id: "i".into(), query: "q".into() }, "waypointer_search_all"),
            (Request::IframeMint { id: "i".into(), module_id: "m".into(), slot: "s".into() }, "iframe_mint"),
            (Request::Subscribe { id: "i".into(), kinds: vec![] }, "subscribe"),
        ];
        for (req, tag) in cases {
            let json = serde_json::to_string(&req).unwrap();
            assert!(
                json.contains(&format!("\"type\":\"{tag}\"")),
                "{tag}: wire tag changed, got {json}"
            );
            // And it round-trips back to the same variant tag.
            let reparsed: Request = serde_json::from_str(&json).unwrap();
            assert!(serde_json::to_string(&reparsed).unwrap().contains(&format!("\"type\":\"{tag}\"")));
        }
    }

    /// The newer `HostCall` variants (added after the original test) must also
    /// survive the JSON wire format with their byte-safe base64 bodies intact.
    #[test]
    fn host_call_post_and_write_round_trip() {
        let write = HostCall::GraphWrite { cypher: "CREATE (n)".into() };
        let json = serde_json::to_string(&write).unwrap();
        assert!(json.contains("\"type\":\"graph_write\""));
        assert!(matches!(serde_json::from_str::<HostCall>(&json).unwrap(), HostCall::GraphWrite { .. }));

        let post = HostCall::NetworkPost {
            url: "https://api.example.com".into(),
            body_b64: "eyJrIjogInYifQ==".into(),
            headers: vec![("Content-Type".into(), "application/json".into())],
        };
        let json = serde_json::to_string(&post).unwrap();
        let back: HostCall = serde_json::from_str(&json).unwrap();
        match back {
            HostCall::NetworkPost { url, body_b64, headers } => {
                assert_eq!(url, "https://api.example.com");
                assert_eq!(body_b64, "eyJrIjogInYifQ==", "the base64 body must survive verbatim");
                assert_eq!(headers, vec![("Content-Type".to_string(), "application/json".to_string())]);
            }
            other => panic!("NetworkPost did not round-trip: {other:?}"),
        }
    }

    /// Every `HostReply` variant round-trips and keeps its wire tag, including
    /// the `Error` reply that carries an `ErrorCode`.
    #[test]
    fn host_reply_variants_round_trip_with_tags() {
        let cases: Vec<(HostReply, &str)> = vec![
            (HostReply::GraphResult { rows: "[]".into() }, "graph_result"),
            (HostReply::NetworkBody { status: 200, body_b64: "AA==".into() }, "network_body"),
            (HostReply::Acked, "acked"),
            (HostReply::Error { code: ErrorCode::Timeout, message: "slow".into() }, "error"),
        ];
        for (reply, tag) in cases {
            let json = serde_json::to_string(&reply).unwrap();
            assert!(json.contains(&format!("\"type\":\"{tag}\"")), "{tag}: tag changed, got {json}");
            let _: HostReply = serde_json::from_str(&json).unwrap();
        }
    }

    /// `ErrorCode` is part of the wire contract; its snake_case names must stay
    /// stable so a daemon and an older client still agree on a denial reason.
    #[test]
    fn error_code_wire_names_are_stable() {
        let cases = [
            (ErrorCode::NotFound, "not_found"),
            (ErrorCode::PermissionDenied, "permission_denied"),
            (ErrorCode::ModuleFailed, "module_failed"),
            (ErrorCode::Timeout, "timeout"),
            (ErrorCode::InvalidRequest, "invalid_request"),
            (ErrorCode::Internal, "internal"),
        ];
        for (code, name) in cases {
            let json = serde_json::to_string(&code).unwrap();
            assert_eq!(json, format!("\"{name}\""), "ErrorCode wire name changed");
            let back: ErrorCode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, code);
        }
    }
}
