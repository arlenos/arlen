//! App identity resolution.
//!
//! Sprint C consolidated the canonical implementation into
//! `sdk/permissions::identity`. This module re-exports the
//! types so existing `knowledge::auth::Authenticator` callsites
//! keep working unchanged.
//!
//! See `docs/architecture/AUTH-CANONICAL.md` section 4.

// `app_id_from_pid` is deliberately NOT re-exported here as of 15 Aug.
//
// This daemon carries `ProtectSystem=strict`, and under that directive a process
// cannot read another's `/proc/<pid>/exe` - measured with one binary in four
// units on one boot: 29 of 31 same-uid links plain, 1 of 25 hardened. So the
// function cannot answer here, and every caller that used it was resolving an
// identity the identity broker had already established.
//
// Leaving it re-exported would make it available to the next person who needs a
// caller's name and reaches for the obvious thing. It is not available, and the
// route is the broker.
pub use arlen_permissions::identity::{
    path_to_app_id, pid_start_time, process_alive, IdentityError,
};
