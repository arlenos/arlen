/// Arlen Module Runtime daemon.
///
/// `arlen-modulesd` hosts every sandboxed module on the system in
/// one of two tiers:
///
///   * **Tier 1 (WASM Components)** for data-only modules (Waypointer
///     providers, Waypointer actions, MCP servers, keybinding
///     profiles). The daemon owns the Wasmtime engine, instantiates a
///     fresh Store per module, and links a capability-gated set of
///     host imports defined in `sdk/module-sdk/wit/host.wit`.
///   * **Tier 2 (Iframes)** for UI-rendering modules (topbar
///     indicators, applets, settings panels). The iframe DOM lives in
///     the desktop-shell webview; the daemon owns the policy (nonce
///     binding, capability checks on postMessage host calls).
///
/// Communication with the shell uses a length-prefixed JSON protocol
/// over a Unix socket at `/run/user/{uid}/arlen/modulesd.sock`.
/// See `socket::protocol` for the wire format.
///
/// Foundation reference: §07 "Apps and Modules", in particular the
/// "Module Runtime" paragraph and Table 08 (crash recovery).
/// Architecture reference: `docs/architecture/module-system.md`.

pub mod error;
pub mod host;
pub mod manager;
pub mod manifest;
pub mod runtime;
pub mod socket;

pub use error::{DaemonError, Result};
pub use manager::Manager;
pub use socket::{Event, Request, Response, SocketServer};
