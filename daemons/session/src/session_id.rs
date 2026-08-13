//! One login is one session, and exactly one thing mints its id.
//!
//! Everything downstream READS it. The compositor and the kernel-layer each used
//! to mint their own uuid when it was absent, which put TWO ids on one login with
//! neither locally wrong - a component is not positioned to know how many sessions
//! a machine has, and the disagreement was the proof. Every app-side producer that
//! emits onto the bus needs the same value, or the graph cannot join a file the
//! user opened to the window they had focused when they opened it.

/// The environment variable carrying the id, read by every producer.
pub const SESSION_ID_VAR: &str = "ARLEN_SESSION_ID";

/// The id for this session: an inherited one if the login path already set it,
/// otherwise a fresh one.
///
/// Inheriting matters: a session re-entered through a path that already minted an
/// id must not mint a second, for exactly the reason two components minting
/// separately was wrong. A blank value is not an id - it is an empty variable
/// someone exported - so it is treated as absent rather than propagated.
///
/// NOT the boot id: two logins in one boot are two sessions.
pub fn session_id(inherited: Option<String>) -> String {
    match inherited {
        Some(id) if !id.trim().is_empty() => id,
        _ => uuid::Uuid::new_v4().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inherited_id_is_kept_so_one_login_never_has_two() {
        assert_eq!(session_id(Some("abc-123".into())), "abc-123");
    }

    #[test]
    fn a_blank_or_absent_value_mints_a_fresh_one() {
        // An exported-but-empty variable is not an id; propagating it would give
        // every producer the same empty string and silently join unrelated
        // sessions.
        for empty in [None, Some(String::new()), Some("   ".to_string())] {
            let id = session_id(empty);
            assert!(!id.trim().is_empty());
            assert_ne!(id, session_id(None), "each mint is its own session");
        }
    }
}
