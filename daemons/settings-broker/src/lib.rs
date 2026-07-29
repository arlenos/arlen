//! Settings Broker: the session daemon that owns WRITES to an app's settings
//! (per-app-settings-plan.md section 3).
//!
//! The architecture is dconf's asymmetric hybrid. Reads bypass this entirely -
//! an app reads its own `config.toml` directly, which is why dconf can describe
//! its service as "only involved in writes" and "stateless… robust against
//! crashes". What must NOT bypass it is a write: the Settings app never edits an
//! app's config file itself, it asks the broker, which serialises the write,
//! validates it against the app's declared schema and scope, applies it
//! atomically, and announces exactly which keys changed.
//!
//! This module is the decision half: given a declared schema and a proposed
//! write, may it proceed. It is pure, so the rule that actually protects the
//! file is testable without a socket, a daemon, or a filesystem.
//!
//! The scope rule is the one worth stating plainly, because it inverts the
//! usual instinct: **the editor enforces scope, not the caller.** VS Code's
//! `configurationEditing` rejects a write aimed at the wrong layer rather than
//! trusting the requester to target the right one. A caller that could pick its
//! own layer could write a `defaults_only` key by simply claiming it may.

pub mod apply;
pub mod decide;

pub use apply::{apply_to_file, ApplyError};
pub use decide::{decide_write, WriteRejection, WriteRequest};
