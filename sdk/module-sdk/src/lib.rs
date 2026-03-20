// module-sdk exposes a restricted subset of os-sdk for third-party modules.
// It re-exports only what external developers need, keeping internal
// os-sdk surfaces unexposed.

pub use os_sdk::event::{EventEmitter, EmitError};
