// module-sdk exposes a restricted subset of os-sdk for third-party modules.
// It re-exports only what external developers need, keeping internal
// os-sdk surfaces unexposed.
//
// Tier 1 modules (WASM components: Waypointer providers, MCP servers,
// keybinding profiles) link against the `host` module's
// capability-gated functions. Tier 2 modules (iframes: topbar,
// settings panels) consume the TypeScript surface in `index.ts`,
// which wraps the same capabilities behind postMessage.

pub mod host;
pub mod waypointer;

pub use host::HostError;
pub use os_sdk::event::{EmitError, EventEmitter};
pub use waypointer::{Action, PluginDescriptor, PluginError, SearchResult, WaypointerPlugin};
