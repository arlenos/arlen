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
//!
//! Resolving a source's glob to concrete files (with the `instance_strategy`), the
//! write engine (`write_strategy` + `verify`), and the Settings render build on
//! this.

pub mod adapter;
pub mod allowlist;
pub mod resolve;

pub use adapter::{
    AdapterError, AdapterManifest, AdapterMeta, FormatName, InstanceStrategy, SettingSpec,
    SettingType, SourceSpec, WriteStrategy, SCHEMA_VERSION,
};
pub use allowlist::{resolve_under_allowlist, AllowlistError, ALLOWED_SUBDIRS};
pub use resolve::{glob_under, resolve, Match, Resolution};
