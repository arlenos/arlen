//! The Terminal-run MCP server (mcp-server-layer.md + ai-act-layer-plan.md): the
//! peer-authenticated surface exposing `run_command` over the AI daemon's MCP
//! transport. It is a thin, separate process so the confined-spawn machinery and a
//! runaway command's resource use stay out of the daemon (the plan reserves the
//! MCP-server pattern for exactly this - the confined sandbox boundary).
//!
//! ## Safety: fail-closed at the consent boundary
//!
//! `run_command` is the sharp edge - opaque, unboundable, un-undoable
//! (`OpaqueCommand`). Its whole safety story is **always-Confirm + confined +
//! output-captured + never-autonomous**. Two layers guard it here:
//!
//! 1. **Peer-auth** ([`os_sdk::mcp::serve_mcp`]): only the AI daemon may connect
//!    (`SO_PEERCRED` + the admitted-app allowlist), so no arbitrary process reaches
//!    this socket.
//! 2. **Per-action consent** ([`authorize_run`]): the pi gate classifies
//!    `run_command` `Confirm` and the consent broker surfaces it, but a compromised
//!    caller could otherwise invoke this server directly. So the server ITSELF
//!    verifies per-action consent at the boundary: the AI-engine daemon mints a
//!    public-key-verifiable Biscuit (ai-act-layer-plan.md "biscuit per-action
//!    tie-in") only after the user approves the Confirm, bound to the exact
//!    command + args, and this server verifies it here against the daemon's
//!    published root public key. A missing token, a token for a different command,
//!    an expired token or a bad signature all refuse - fail-closed, so no command
//!    runs without a verified, per-action confirmation. (The remaining wiring is
//!    pi threading the daemon-minted token into the call's `consent` argument.)

use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use os_sdk::mcp::rmcp;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};

use crate::run::{run_confined, RunRequest, DEFAULT_TIMEOUT};
use arlen_confiner::NetworkPolicy;

/// Well-known socket id for the system Terminal-run server. Resolves to
/// `$XDG_RUNTIME_DIR/arlen/mcp/system.terminal-run.sock` via the os-sdk helper.
pub const SERVER_ID: &str = "system.terminal-run";

/// The one tool: run a single confirmed command in the sandbox.
pub const RUN_TOOL: &str = "run_command";

/// The hard ceiling on a command's wall-clock budget (a caller can request less).
const MAX_TIMEOUT: Duration = Duration::from_secs(120);

/// The Terminal-run MCP server. Stateless; a fresh handle is built per connection.
#[derive(Clone, Default)]
pub struct TerminalRunMcp;

impl TerminalRunMcp {
    /// Build a server handle.
    pub fn new() -> Self {
        Self
    }

    /// The `run_command` tool schema.
    fn run_tool() -> Tool {
        let schema: JsonObject = serde_json::from_value(serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The program to run (no shell; resolved on PATH)." },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Arguments, each a separate string (never a shell line)."
                },
                "timeout_ms": { "type": "integer", "description": "Wall-clock budget in ms (capped)." },
                "consent": { "type": "string", "description": "The per-action consent token proving the user confirmed THIS command (minted by the AI-engine daemon on approval)." }
            },
            "required": ["command", "consent"]
        }))
        .expect("the static run_command schema is a valid JSON object");
        Tool::new_with_raw(
            RUN_TOOL.to_owned(),
            Some(Cow::Borrowed(
                "Run a single confirmed command in a confined sandbox (no host write, no network, no privilege) and return its captured output. Always requires prior user confirmation; never autonomous.",
            )),
            Arc::new(schema),
        )
    }
}

/// The parsed, validated `run_command` arguments.
struct RunArgs {
    command: String,
    args: Vec<String>,
    timeout: Duration,
    /// The per-action consent token (a base64 Biscuit) the daemon minted on
    /// approval; verified against the daemon's published root public key before any
    /// command runs.
    consent: String,
}

/// Parse + validate the `run_command` arguments. A missing/empty command or a
/// non-string arg is a clean tool error, never a guess.
fn parse_run_args(arguments: Option<&JsonObject>) -> Result<RunArgs, McpError> {
    let map = arguments.ok_or_else(|| {
        McpError::invalid_request("run_command requires a 'command' argument", None)
    })?;
    let command = map
        .get("command")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| McpError::invalid_request("run_command needs a non-empty string 'command'", None))?
        .to_string();
    let args = match map.get("args") {
        None => Vec::new(),
        Some(v) => {
            let arr = v
                .as_array()
                .ok_or_else(|| McpError::invalid_request("'args' must be an array of strings", None))?;
            let mut out = Vec::with_capacity(arr.len());
            for a in arr {
                let s = a.as_str().ok_or_else(|| {
                    McpError::invalid_request("every 'args' entry must be a string", None)
                })?;
                out.push(s.to_string());
            }
            out
        }
    };
    let timeout = match map.get("timeout_ms").and_then(|v| v.as_u64()) {
        Some(ms) => Duration::from_millis(ms).min(MAX_TIMEOUT),
        None => DEFAULT_TIMEOUT,
    };
    let consent = map
        .get("consent")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| McpError::invalid_request("run_command needs a non-empty 'consent' token", None))?
        .to_string();
    Ok(RunArgs { command, args, timeout, consent })
}

/// Verify per-action consent for a `run_command` at the MCP boundary. FAIL-CLOSED
/// everywhere: reads the AI-engine daemon's published root PUBLIC key (the shared
/// rendezvous file) and verifies the presented biscuit binds THIS exact command +
/// args and has not expired. A missing/malformed key file, a token that does not
/// authorize this command, an expired token, or a bad signature all refuse - so the
/// server runs nothing without a verified, per-action user confirmation. The daemon
/// mints the token only after the consent broker approves the run_command Confirm.
fn authorize_run(args: &RunArgs) -> Result<(), String> {
    let pub_path = arlen_run_consent_token::published_public_key_path()
        .ok_or_else(|| "no state dir to read the consent public key from".to_string())?;
    let hex = std::fs::read_to_string(&pub_path)
        .map_err(|e| format!("consent public key unavailable ({}): {e}", pub_path.display()))?;
    let key = arlen_run_consent_token::public_key_from_hex(&hex)
        .map_err(|e| format!("consent public key malformed: {e}"))?;
    authorize_run_with_key(args, &key)
}

/// The consent-verify core, over a resolved public key (so it is hermetically
/// testable without touching the rendezvous file). Verifies the presented biscuit
/// binds THIS exact command + args and has not expired at the current wall clock.
fn authorize_run_with_key(
    args: &RunArgs,
    key: &arlen_run_consent_token::PublicKey,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "system clock is before the unix epoch".to_string())?
        .as_secs() as i64;
    match arlen_run_consent_token::verify_run_consent(
        &args.consent,
        key,
        &args.command,
        &args.args,
        now,
    ) {
        Ok(true) => Ok(()),
        Ok(false) => {
            Err("the consent token does not authorize this command, or it has expired".to_string())
        }
        Err(e) => Err(format!("consent token invalid: {e}")),
    }
}

/// The system directories a confined command may READ: enough to resolve and run
/// a binary (interpreter, shared libraries, `ld.so.cache`, `/etc/passwd`), and
/// nothing else.
///
/// This is deliberately NOT the host root. Binding `/` read-only looks harmless -
/// the command still cannot write, reach the network or gain privilege - but a
/// read-only bind is not an isolation boundary for two surfaces that matter:
///
/// * **IPC.** A read-only bind does not stop `connect()`: the kernel returns
///   `-EROFS` for `MAY_WRITE` only on regular files, directories and symlinks, so
///   a pathname AF_UNIX socket stays connectable. `--unshare-net` isolates only
///   ABSTRACT sockets; pathname sockets are filesystem objects and are not
///   namespaced by a netns. With `/run` visible, a confined command reaches
///   `/run/user/$UID/bus` - and the session bus authenticates any same-uid peer -
///   so `StartTransientUnit` would spawn an UNCONFINED process on the host,
///   defeating confined + always-Confirm + never-autonomous in one step. Every
///   arlen daemon socket under `$XDG_RUNTIME_DIR/arlen/` is reachable the same way.
/// * **Secrets.** `$HOME` holds the AI engine's own run-consent root key, the
///   capsule and undo signing keys, the audit HMAC key, `~/.ssh`, browser cookie
///   stores. Reading the consent root alone would let an attacker mint valid
///   consent for ARBITRARY argv, making every future confirmation meaningless -
///   and the captured stdout returns straight into the model's context.
///
/// Excluding `/home`, `/root`, `/run`, `/var` and `/tmp` (a fresh tmpfs) closes
/// both. A command that legitimately needs a user path should get it passed
/// explicitly and covered by the consent digest, never by a blanket root bind.
///
/// Missing entries are filtered out because `bwrap` fails the whole launch on a
/// bind source that does not exist (on a merged-`/usr` system `/bin`, `/sbin`,
/// `/lib` and `/lib64` are symlinks into `/usr` and may legitimately be absent).
fn system_read_roots() -> Vec<PathBuf> {
    ["/usr", "/etc", "/bin", "/sbin", "/lib", "/lib64"]
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

/// A per-call writable scratch dir, UNIQUE per call and removed on drop. Two
/// concurrent runs must not share a workdir (one command's scratch must never be
/// visible to another), and each command gets a fresh empty dir; `Drop` cleans it
/// up so a command's scratch never leaks into the next.
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    /// Create a unique empty scratch dir under `$XDG_RUNTIME_DIR/arlen/terminal-run`.
    fn create() -> std::io::Result<ScratchDir> {
        static N: AtomicU64 = AtomicU64::new(0);
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path =
            base.join("arlen").join("terminal-run").join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(&path)?;
        Ok(ScratchDir { path })
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

impl ServerHandler for TerminalRunMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Run a single, previously-confirmed command in a confined sandbox and return its captured output. The command runs with no host write access, no network, and no privilege. Every run requires prior per-action user confirmation; nothing runs autonomously.",
        )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(vec![Self::run_tool()]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let tool = request.name.to_string();
        if tool != RUN_TOOL {
            return Err(McpError::invalid_request(format!("unknown tool: {tool}"), None));
        }
        let args = parse_run_args(request.arguments.as_ref())?;

        // FAIL-CLOSED consent boundary FIRST: nothing runs without verified consent.
        if let Err(why) = authorize_run(&args) {
            return Ok(CallToolResult::error(vec![Content::text(format!(
                "run_command refused: {why}"
            ))]));
        }

        // Reached only once consent is wired: run the confined command in a fresh,
        // isolated, per-call scratch dir (cleaned up when `scratch` drops) + return
        // the captured output. The read surface is the curated system set, NOT the
        // host root - see `system_read_roots`.
        let scratch = match ScratchDir::create() {
            Ok(s) => s,
            Err(e) => {
                return Ok(CallToolResult::error(vec![Content::text(format!(
                    "run_command: could not prepare the scratch dir: {e}"
                ))]))
            }
        };
        let req = RunRequest {
            command: args.command,
            args: args.args,
            read_only_roots: system_read_roots(),
            workdir: scratch.path.clone(),
            network: NetworkPolicy::None,
            timeout: args.timeout,
        };
        let result = run_confined(&req).await;
        drop(scratch); // clean the per-call scratch before returning.
        match result {
            Ok(outcome) => {
                let json = serde_json::to_string(&outcome).unwrap_or_else(|_| "{}".to_string());
                Ok(CallToolResult::success(vec![Content::text(json)]))
            }
            Err(e) => Ok(CallToolResult::error(vec![Content::text(format!(
                "run_command failed: {e}"
            ))])),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The read surface must never expose the host root or any directory holding a
    /// connectable socket or a secret: `/run` (the session bus + every arlen daemon
    /// socket - a read-only bind does not stop `connect()`, and a netns does not
    /// bound pathname AF_UNIX) or `$HOME` (the consent root key, whose disclosure
    /// would let an attacker mint consent for arbitrary argv). Pins the invariant
    /// the seccomp socket entries rely on.
    #[test]
    fn the_read_surface_excludes_ipc_and_secret_directories() {
        let roots = system_read_roots();
        assert!(!roots.is_empty(), "a command needs some system dirs to run");
        for r in &roots {
            let p = r.to_string_lossy();
            assert_ne!(p, "/", "the host root is never the read surface");
            for forbidden in ["/run", "/var", "/home", "/root", "/tmp", "/proc", "/sys"] {
                assert!(
                    !p.starts_with(forbidden),
                    "{p} exposes {forbidden}, which holds sockets or secrets"
                );
            }
        }
        // The set must still be usable: a binary and its loader have to resolve.
        assert!(roots.iter().any(|r| r.ends_with("usr")), "no /usr: nothing could exec");
    }

    fn args(command: &str) -> JsonObject {
        serde_json::from_value(
            serde_json::json!({ "command": command, "args": ["-la"], "consent": "tok" }),
        )
        .unwrap()
    }

    #[test]
    fn parse_requires_a_non_empty_command() {
        assert!(parse_run_args(Some(&args("ls"))).is_ok());
        let empty: JsonObject =
            serde_json::from_value(serde_json::json!({ "command": "", "consent": "t" })).unwrap();
        assert!(parse_run_args(Some(&empty)).is_err());
        let missing: JsonObject =
            serde_json::from_value(serde_json::json!({ "args": [], "consent": "t" })).unwrap();
        assert!(parse_run_args(Some(&missing)).is_err());
        assert!(parse_run_args(None).is_err());
    }

    #[test]
    fn parse_requires_a_consent_token() {
        // run_command is the sharp edge: it cannot even be parsed without a consent
        // token, so no command reaches the runner without one.
        let no_consent: JsonObject =
            serde_json::from_value(serde_json::json!({ "command": "ls" })).unwrap();
        assert!(parse_run_args(Some(&no_consent)).is_err());
        let empty_consent: JsonObject =
            serde_json::from_value(serde_json::json!({ "command": "ls", "consent": "" })).unwrap();
        assert!(parse_run_args(Some(&empty_consent)).is_err());
    }

    #[test]
    fn parse_rejects_non_string_args() {
        let bad: JsonObject = serde_json::from_value(
            serde_json::json!({ "command": "ls", "args": [1, 2], "consent": "t" }),
        )
        .unwrap();
        assert!(parse_run_args(Some(&bad)).is_err());
    }

    #[test]
    fn parse_clamps_the_timeout() {
        let long: JsonObject = serde_json::from_value(
            serde_json::json!({ "command": "ls", "timeout_ms": 9_999_999, "consent": "t" }),
        )
        .unwrap();
        assert_eq!(parse_run_args(Some(&long)).unwrap().timeout, MAX_TIMEOUT);
    }

    /// A RunArgs carrying a real minted consent token for `(command, args)`.
    fn run_args_with_consent(
        root: &biscuit_auth::KeyPair,
        command: &str,
        cmd_args: &[&str],
    ) -> RunArgs {
        let owned: Vec<String> = cmd_args.iter().map(|s| s.to_string()).collect();
        let consent =
            arlen_run_consent_token::mint_run_consent(root, command, &owned, 4_102_444_800).unwrap();
        RunArgs {
            command: command.to_string(),
            args: owned,
            timeout: DEFAULT_TIMEOUT,
            consent,
        }
    }

    #[test]
    fn a_valid_consent_token_authorizes_exactly_its_command() {
        let root = biscuit_auth::KeyPair::new();
        let a = run_args_with_consent(&root, "ls", &["-la"]);
        assert!(authorize_run_with_key(&a, &root.public()).is_ok());
    }

    #[test]
    fn a_consent_token_for_another_command_is_refused() {
        // A token minted for `ls -la` cannot authorize a different command, even under
        // the correct key: the digest binds the exact argv.
        let root = biscuit_auth::KeyPair::new();
        let token = arlen_run_consent_token::mint_run_consent(
            &root,
            "ls",
            &["-la".to_string()],
            4_102_444_800,
        )
        .unwrap();
        let evil = RunArgs {
            command: "rm".to_string(),
            args: vec!["-rf".to_string(), "/".to_string()],
            timeout: DEFAULT_TIMEOUT,
            consent: token,
        };
        assert!(authorize_run_with_key(&evil, &root.public()).is_err());
    }

    #[test]
    fn a_token_signed_by_the_wrong_key_is_refused() {
        let root = biscuit_auth::KeyPair::new();
        let attacker = biscuit_auth::KeyPair::new();
        let a = run_args_with_consent(&root, "ls", &["-la"]);
        // Verifying under a different public key is a signature failure -> refused.
        assert!(authorize_run_with_key(&a, &attacker.public()).is_err());
    }

    #[test]
    fn a_garbage_consent_token_is_refused() {
        let root = biscuit_auth::KeyPair::new();
        let a = RunArgs {
            command: "ls".to_string(),
            args: vec!["-la".to_string()],
            timeout: DEFAULT_TIMEOUT,
            consent: "not-a-biscuit".to_string(),
        };
        assert!(authorize_run_with_key(&a, &root.public()).is_err());
    }

    #[test]
    fn scratch_dirs_are_unique_and_cleaned_on_drop() {
        let a = ScratchDir::create().unwrap();
        let b = ScratchDir::create().unwrap();
        assert_ne!(a.path, b.path, "each call gets its own scratch dir");
        assert!(a.path.is_dir() && b.path.is_dir(), "both created");
        let a_path = a.path.clone();
        drop(a);
        assert!(!a_path.exists(), "the scratch is removed on drop, no leak into the next run");
        assert!(b.path.is_dir(), "the other run's scratch is untouched");
    }
}
