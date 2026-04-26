/// Wire protocol re-export.
///
/// The actual types live in the `modulesd-proto` crate so they can be
/// consumed by clients (desktop-shell, settings, forage) without
/// pulling Wasmtime as a transitive dependency. This re-export keeps
/// the in-daemon import path stable.

pub use modulesd_proto::*;
