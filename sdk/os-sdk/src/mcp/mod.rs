//! MCP server boilerplate for first-party apps.
//!
//! A Arlen app exposes its capabilities as MCP tools. The tool
//! definitions and handlers are written with `rmcp`'s `#[tool]` /
//! `#[tool_router]` / `#[tool_handler]` macros, which this module
//! re-exports so an app does not depend on `rmcp` directly. What
//! this module adds is the socket side: it places the per-app MCP
//! socket under `$XDG_RUNTIME_DIR/arlen/mcp/`, binds it with the
//! right mode, peer-authenticates every connection, and serves a
//! fresh handler instance on each admitted one.
//!
//! Peer authentication closes the same-UID gap that socket mode
//! `0600` leaves open: `0600` keeps other Unix users out, but every
//! process of the logged-in user can still reach the socket. Phase 9
//! has exactly one legitimate MCP client, the AI daemon, so each
//! accepted connection is identified via `SO_PEERCRED` and only the
//! daemon is served. Every other caller is logged and dropped.
//!
//! Minimal app:
//!
//! ```ignore
//! use os_sdk::mcp::{serve_mcp, rmcp};
//! use rmcp::{ServerHandler, tool, tool_handler, tool_router};
//! use rmcp::handler::server::router::tool::ToolRouter;
//!
//! #[derive(Clone)]
//! struct Files { tool_router: ToolRouter<Self> }
//!
//! #[tool_router(router = tool_router)]
//! impl Files {
//!     fn new() -> Self { Self { tool_router: Self::tool_router() } }
//!     #[tool(name = "list_directory")]
//!     async fn list_directory(&self) -> Result<String, String> {
//!         Ok("...".into())
//!     }
//! }
//!
//! #[tool_handler(router = self.tool_router)]
//! impl ServerHandler for Files {}
//!
//! // in the app's async runtime:
//! serve_mcp("com.arlen.files", Files::new).await?;
//! ```

pub use rmcp;

use std::path::{Path, PathBuf};

use arlen_permissions::ConnectionAuth;
use rmcp::ServiceExt;
use tokio::net::UnixListener;

/// Resolved `app_id` of the canonically-installed AI daemon, the
/// sole MCP client in Phase 9. `arlen-permissions` maps the
/// daemon's install path to this id; see `identity::path_to_app_id`.
/// The one admitted MCP client: the AI engine daemon.
///
/// This is `ai-agent`, not `ai-engine-daemon`, because an app id names a ROLE and
/// not a binary: `identity::path_to_app_id` maps the engine's canonical install
/// path (`/usr/lib/arlen/libexec/arlen-ai-engine-daemon`) onto the `ai-agent`
/// principal, so it reuses the retired agent's go-live profile and audit ADMITTED
/// entry. It was `ai-daemon` until the pi cutover retired that daemon; since
/// nothing resolves to `ai-daemon` any more (no crate, no packaged binary), keeping
/// it here would have admitted a principal that cannot exist while refusing the one
/// that does - in RELEASE only, because the `dev.*` branch below masks it in every
/// debug build.
const AI_ENGINE_APP_ID: &str = "ai-agent";

/// Whether a peer that resolved to `app_id` may open MCP connections
/// to a first-party app's server.
///
/// There is one MCP client, the AI engine daemon, so only it is
/// admitted. In debug builds every component runs straight from a
/// cargo target directory and resolves to a `dev.*` id (the engine
/// included), so those are admitted too for local development; the
/// branch compiles out of release builds.
fn caller_is_admitted(app_id: &str) -> bool {
    if app_id == AI_ENGINE_APP_ID {
        return true;
    }
    cfg!(debug_assertions) && app_id.starts_with("dev.")
}

/// Errors raised while setting up or running an MCP server socket.
#[derive(Debug, thiserror::Error)]
pub enum McpServeError {
    /// The socket directory, bind, or permission setup failed.
    #[error("mcp socket setup failed: {0}")]
    Socket(String),
    /// The accept loop failed.
    #[error("mcp accept loop failed: {0}")]
    Accept(String),
}

/// Resolve the per-app MCP socket path:
/// `$XDG_RUNTIME_DIR/arlen/mcp/{app_id}.sock`, falling back to
/// `/run/arlen/mcp/{app_id}.sock` when the runtime dir is unset.
pub fn mcp_socket_path(app_id: &str) -> PathBuf {
    mcp_runtime_dir().join(format!("{app_id}.sock"))
}

/// Resolve the per-*module* MCP socket path:
/// `$XDG_RUNTIME_DIR/arlen/mcp/modules/{module_id}.sock`. Tier-1
/// `mcp.server` modules hosted by `arlen-modulesd` live one
/// directory below first-party app sockets so the two namespaces
/// can never collide. Both modulesd (which binds the socket) and the
/// AI daemon (which connects to it) resolve the path through here so
/// the convention has a single source of truth.
///
/// The caller must reject ids that fail [`is_safe_module_id`] first:
/// the id is formatted straight into the path, so a `/` or `..` in
/// it would escape the modules directory.
pub fn mcp_module_socket_path(module_id: &str) -> PathBuf {
    mcp_runtime_dir()
        .join("modules")
        .join(format!("{module_id}.sock"))
}

/// Whether a module id is safe to embed in an MCP socket filename.
///
/// [`mcp_module_socket_path`] formats the id straight into a path,
/// so an id carrying a `/` (or any non-reverse-domain character)
/// could place or resolve the socket outside the modules directory.
/// modulesd checks this before binding, and the AI daemon's
/// discovery checks it before connecting — neither trusts an id it
/// did not validate. Accepts the reverse-domain charset only.
pub fn is_safe_module_id(module_id: &str) -> bool {
    !module_id.is_empty()
        && module_id.len() <= 128
        && module_id != "."
        && module_id != ".."
        // The `system.` prefix is reserved for Arlen-shipped system MCP
        // servers (e.g. `system.knowledge`). A module is registered in the
        // AI daemon's client under its raw id, and a new connection replaces
        // any existing one for that id, so a module allowed to claim
        // `system.knowledge` could shadow the authenticated read-only system
        // server. Reject the namespace here, where both the host (modulesd,
        // which binds the socket) and the AI daemon (which connects to it)
        // validate the id.
        && !module_id.starts_with("system.")
        && module_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// `$XDG_RUNTIME_DIR/arlen/mcp/`, falling back to
/// `/run/arlen/mcp/` when the runtime dir is unset.
fn mcp_runtime_dir() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run"));
    base.join("arlen").join("mcp")
}

/// Bind the app's MCP socket and serve it. Convenience wrapper over
/// [`serve_mcp_at`] that resolves the canonical per-app socket path.
pub async fn serve_mcp<S, F>(app_id: &str, make_handler: F) -> Result<(), McpServeError>
where
    S: rmcp::ServerHandler + Send + 'static,
    F: Fn() -> S + Send + 'static,
{
    serve_mcp_at(&mcp_socket_path(app_id), make_handler).await
}

/// Bind an MCP server socket at an explicit path and serve a fresh
/// `make_handler()` instance on every admitted connection.
///
/// Runs until the accept loop errors; an app spawns this as a
/// long-lived task. Each connection is peer-authenticated via
/// `SO_PEERCRED`: only the AI daemon is served, every other caller
/// is logged and dropped (see [`caller_is_admitted`]).
///
/// If the socket path is already bound by a live server the call
/// fails rather than clobbering it, so a double-launched app cannot
/// silently hijack the first instance's socket. A path with nothing
/// listening behind it is a stale leftover and is cleared first.
///
/// The socket is mode 0600. Combined with peer auth that gives two
/// layers: `0600` excludes other Unix users, peer auth excludes
/// other processes of the same user.
///
/// A caller that needs to know the bind succeeded before acting
/// (e.g. announcing the socket on a discovery channel) uses
/// [`bind_mcp_socket`] + [`serve_mcp_listener`] directly: `serve_mcp_at`
/// is the two composed for callers that do not.
pub async fn serve_mcp_at<S, F>(
    socket_path: &Path,
    make_handler: F,
) -> Result<(), McpServeError>
where
    S: rmcp::ServerHandler + Send + 'static,
    F: Fn() -> S + Send + 'static,
{
    let listener = bind_mcp_socket(socket_path)?;
    serve_mcp_listener(listener, make_handler).await
}

/// Bind (only) the MCP server socket at `socket_path`, mode 0600.
///
/// Returns the bound [`UnixListener`] on success. A successful return
/// is proof this process owns the socket: the caller may safely
/// announce it. If a live server already holds the path the call
/// fails rather than clobbering it; a stale leftover is cleared
/// first. Pair with [`serve_mcp_listener`].
pub fn bind_mcp_socket(socket_path: &Path) -> Result<UnixListener, McpServeError> {
    use std::os::unix::fs::PermissionsExt;

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| McpServeError::Socket(format!("create dir: {e}")))?;
    }
    // A leftover socket from a crashed run would make bind fail and
    // must be cleared, but a socket with a live server behind it
    // must not be: removing it would silently hijack that server's
    // name when an app is launched twice. Probe before removing.
    if socket_path.exists() {
        match std::os::unix::net::UnixStream::connect(socket_path) {
            Ok(_) => {
                return Err(McpServeError::Socket(format!(
                    "{} is already served by a live MCP server",
                    socket_path.display()
                )));
            }
            Err(_) => {
                // Nothing accepts connections: the path is a stale
                // socket or a leftover file. Safe to clear.
                let _ = std::fs::remove_file(socket_path);
            }
        }
    }

    let listener = UnixListener::bind(socket_path).map_err(|e| {
        McpServeError::Socket(format!("bind {}: {e}", socket_path.display()))
    })?;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| McpServeError::Socket(format!("chmod: {e}")))?;
    Ok(listener)
}

/// Serve an already-[`bind_mcp_socket`]-bound listener: peer-auth
/// every connection and hand admitted ones a fresh `make_handler()`
/// instance. Runs until the accept loop errors.
pub async fn serve_mcp_listener<S, F>(
    listener: UnixListener,
    make_handler: F,
) -> Result<(), McpServeError>
where
    S: rmcp::ServerHandler + Send + 'static,
    F: Fn() -> S + Send + 'static,
{
    // SAFETY: getuid() is always successful and has no preconditions.
    let caller_uid = unsafe { libc::getuid() };

    tracing::info!("mcp server listening");
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| McpServeError::Accept(e.to_string()))?;

        // Identify the peer before serving anything. A connection
        // whose identity cannot be resolved, or that does not belong
        // to an admitted MCP client, is dropped without a handshake.
        let auth = match ConnectionAuth::extract_from(&stream, caller_uid) {
            Ok(auth) => auth,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "mcp connection rejected: peer identity unresolved"
                );
                continue;
            }
        };
        if !caller_is_admitted(auth.app_id()) {
            tracing::warn!(
                caller = %auth.app_id(),
                pid = auth.pid(),
                "mcp connection rejected: caller is not an admitted MCP client"
            );
            continue;
        }

        let handler = make_handler();
        tokio::spawn(async move {
            match handler.serve(stream).await {
                Ok(server) => {
                    let _ = server.waiting().await;
                }
                Err(err) => {
                    tracing::warn!(error = %err, "mcp connection handshake failed");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::handler::server::router::tool::ToolRouter;
    use rmcp::{tool, tool_handler, tool_router, ServerHandler};

    #[test]
    fn socket_path_uses_runtime_dir() {
        // The path joins the per-app sock under arlen/mcp/.
        let p = mcp_socket_path("com.example.files");
        let s = p.to_string_lossy();
        assert!(s.ends_with("arlen/mcp/com.example.files.sock"), "{s}");
    }

    #[test]
    fn module_socket_path_is_under_mcp_modules() {
        let s = mcp_module_socket_path("com.example.notes")
            .to_string_lossy()
            .into_owned();
        assert!(s.ends_with("arlen/mcp/modules/com.example.notes.sock"), "{s}");
    }

    #[test]
    fn is_safe_module_id_rejects_path_escapes() {
        // Reverse-domain ids are accepted.
        assert!(is_safe_module_id("com.example.notes"));
        assert!(is_safe_module_id("org.arlen.knowledge-mcp"));
        // Anything that could escape the modules directory is not.
        assert!(!is_safe_module_id(""));
        assert!(!is_safe_module_id("."));
        assert!(!is_safe_module_id(".."));
        assert!(!is_safe_module_id("a/b"));
        assert!(!is_safe_module_id("../../etc/cron.d/x"));
        assert!(!is_safe_module_id("com.example/../escape"));
        assert!(!is_safe_module_id("has space"));
        assert!(!is_safe_module_id(&"x".repeat(200)));
        // The system.* namespace is reserved for Arlen system servers, so a
        // module can never claim a well-known system id and shadow it.
        assert!(!is_safe_module_id("system.knowledge"));
        assert!(!is_safe_module_id("system.anything"));
    }

    #[derive(Clone)]
    struct DemoServer {
        tool_router: ToolRouter<Self>,
    }

    #[tool_router(router = tool_router)]
    impl DemoServer {
        fn new() -> Self {
            Self {
                tool_router: Self::tool_router(),
            }
        }

        #[tool(name = "ping")]
        async fn ping(&self) -> Result<String, String> {
            Ok("pong".to_string())
        }
    }

    #[tool_handler(router = self.tool_router)]
    impl ServerHandler for DemoServer {}

    #[tokio::test]
    async fn served_socket_is_reachable_and_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir()
            .join(format!("arlen-mcp-srv-{}-{unique}", std::process::id()));
        let socket = dir.join("demo.sock");

        let socket_for_task = socket.clone();
        let server = tokio::spawn(async move {
            let _ = serve_mcp_at(&socket_for_task, DemoServer::new).await;
        });

        // Wait for the socket to appear.
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(socket.exists(), "socket was not bound");

        let mode = std::fs::metadata(&socket).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "socket must be mode 0600");

        // A client connects, runs the handshake, and lists the tool.
        // The client is this test process; running from a cargo
        // target directory it resolves to a `dev.*` id, which
        // `caller_is_admitted` admits in debug builds.
        let stream = tokio::net::UnixStream::connect(&socket).await.unwrap();
        let client = ().serve(stream).await.expect("client handshake");
        let tools = client.list_all_tools().await.expect("list tools");
        assert!(
            tools.iter().any(|t| t.name == "ping"),
            "ping tool not exposed"
        );

        server.abort();
    }

    #[test]
    fn caller_admission_is_restricted_to_the_ai_engine() {
        // The AI engine daemon's principal is always admitted.
        assert!(caller_is_admitted("ai-agent"));
        // The retired pre-pi daemon is NOT: nothing resolves to it any more.
        assert!(!caller_is_admitted("ai-daemon"));
        // Arbitrary same-UID apps are not, regardless of name.
        assert!(!caller_is_admitted("com.example.files"));
        assert!(!caller_is_admitted("notification-daemon"));
        assert!(!caller_is_admitted(""));
        // Debug builds additionally admit cargo-run `dev.*` ids so a
        // local dev session works; release builds admit none of them.
        assert_eq!(caller_is_admitted("dev.arlen-ai-engine-daemon"), cfg!(debug_assertions));
    }

    /// The admission id and the identity resolver must name the SAME principal.
    /// They are separate constants in separate crates, so a rename on either side
    /// silently refuses every MCP connection - and ONLY in release, because the
    /// `dev.*` branch masks it in every debug build and test run. That is exactly
    /// how this broke: the engine's canonical path resolves to `ai-agent` while the
    /// admission list still named the retired `ai-daemon`. Pin the pair.
    #[test]
    fn the_admitted_id_matches_what_the_resolver_gives_the_engine_binary() {
        let engine = std::path::PathBuf::from("/usr/lib/arlen/libexec/arlen-ai-engine-daemon");
        let resolved = arlen_permissions::identity::path_to_app_id(&engine)
            .expect("the engine's canonical install path resolves");
        assert_eq!(
            resolved, AI_ENGINE_APP_ID,
            "the MCP admission id and the identity resolver disagree: the engine \
             resolves to '{resolved}' but only '{AI_ENGINE_APP_ID}' is admitted, so \
             every MCP connection would be refused in a release build"
        );
        assert!(caller_is_admitted(&resolved));
    }

    /// A unique, non-existent socket path under a fresh temp dir.
    fn temp_socket(tag: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("arlen-mcp-{tag}-{}-{unique}", std::process::id()))
            .join("demo.sock")
    }

    #[tokio::test]
    async fn live_socket_is_not_clobbered_by_a_second_server() {
        let socket = temp_socket("live");

        let socket_for_task = socket.clone();
        let server = tokio::spawn(async move {
            let _ = serve_mcp_at(&socket_for_task, DemoServer::new).await;
        });
        for _ in 0..100 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(socket.exists(), "first server did not bind");

        // A second server on the same path must refuse rather than
        // unlink the live socket and take over its name.
        let err = serve_mcp_at(&socket, DemoServer::new)
            .await
            .expect_err("second server must refuse a live socket");
        assert!(matches!(err, McpServeError::Socket(_)), "got: {err:?}");

        server.abort();
    }

    #[tokio::test]
    async fn stale_socket_is_replaced() {
        let socket = temp_socket("stale");
        std::fs::create_dir_all(socket.parent().unwrap()).unwrap();

        // Bind then drop a listener: the socket file stays on disk
        // with nothing listening behind it. That is the stale case.
        {
            let _stale = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        }
        assert!(socket.exists(), "stale socket file should remain on disk");

        let socket_for_task = socket.clone();
        let server = tokio::spawn(async move {
            let _ = serve_mcp_at(&socket_for_task, DemoServer::new).await;
        });

        // The stale path is cleared and rebound, so a client reaches
        // the new server.
        let mut connected = None;
        for _ in 0..100 {
            if let Ok(stream) = tokio::net::UnixStream::connect(&socket).await {
                connected = Some(stream);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let stream = connected.expect("server did not rebind the stale socket");
        let client = ().serve(stream).await.expect("client handshake");
        let tools = client.list_all_tools().await.expect("list tools");
        assert!(tools.iter().any(|t| t.name == "ping"));

        server.abort();
    }
}
