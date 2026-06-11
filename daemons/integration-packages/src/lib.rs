//! Arlen integration packages: the adapter interpreter (integration-packages-plan.md IP-R2).
//!
//! An Integration Package can carry a declarative, code-free Settings ADAPTER: a
//! manifest that names an existing app's config files and the settings to expose
//! over them, so the privileged Settings app can present a native config UI for an
//! app that never integrated with the platform. The adapter is UNTRUSTED community
//! data; Settings does all I/O (via `arlen-config-format`), the adapter runs no
//! code, and this crate is the interpreter that makes that safe:
//!
//! - [`adapter`] parses the manifest into a typed model and validates it
//!   fail-closed.
//! - [`allowlist`] confines every source path to the user-config allowlist, so an
//!   adapter can never name a system file (the declared-path half of the
//!   containment; cap-std closes the access-time symlink half).
//! - [`resolve`] turns a source's glob into concrete files under that confinement
//!   and applies the `instance_strategy`.
//! - [`write`] decides whether an edit may be written now (the `write_strategy`).
//! - [`exec`] is the interpreter actually doing the adapter's work over a resolved
//!   file: reading a setting's live value to display and producing the verified,
//!   format-preserving candidate text for an edit. The privileged Settings app
//!   does the final file write.

pub mod adapter;
pub mod allowlist;
pub mod exec;
pub mod resolve;
pub mod write;

pub use adapter::{
    AdapterError, AdapterManifest, AdapterMeta, FormatName, InstanceStrategy, SettingSpec,
    SettingType, SourceSpec, WriteStrategy, SCHEMA_VERSION,
};
pub use allowlist::{resolve_under_allowlist, AllowlistError, ALLOWED_SUBDIRS};
pub use exec::{prepare_edit, prepare_remove, read_setting, read_text_confined, ExecError};
pub use resolve::{
    confined_root, glob_confined, glob_under, resolve, GlobError, Match, Resolution,
};
pub use write::{comm_matches, write_gate, AppPresence, ProcAppPresence, WriteGate};
