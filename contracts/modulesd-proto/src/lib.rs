/// Wire protocol between desktop-shell and `lunaris-modulesd`.
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
    /// `~/.config/lunaris/modules.toml`.
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
}
