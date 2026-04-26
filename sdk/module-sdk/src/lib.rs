// module-sdk exposes a restricted subset of os-sdk for third-party modules.
// It re-exports only what external developers need, keeping internal
// os-sdk surfaces unexposed.

pub mod waypointer;

pub use os_sdk::event::{EmitError, EventEmitter};
pub use waypointer::{Action, PluginDescriptor, PluginError, SearchResult, WaypointerPlugin};
