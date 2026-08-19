//! Is the knowledge daemon there at all?
//!
//! Every read in this app dials the same socket, and every one of them used to
//! report the same sentence when it could not: "cannot read your X right now".
//! True, and useless on the machine where it matters most - one that has not
//! started the service, which is the normal state of a fresh install rather than
//! a fault. The reader is told something failed and given nowhere to go.
//!
//! An absent socket is not a failed read, and the difference is one `exists()`.
//! This is that check, in one place, so the five surfaces agree about what they
//! are looking at.

/// The marker a command returns when the daemon is not running.
///
/// A token rather than a sentence: the wording belongs to the page, where it is
/// translated, and a backend that shipped English prose would be a string the
/// German build could not replace.
pub const NOT_RUNNING: &str = "knowledge-daemon-not-running";

/// The daemon's socket, and whether anything is listening on it.
///
/// `Err(NOT_RUNNING)` when the socket is absent, so a caller can return it
/// straight to the frontend with `?`.
pub fn socket_or_absent() -> Result<std::path::PathBuf, String> {
    let socket = os_sdk::runtime::socket_path("ARLEN_KNOWLEDGE_SOCKET", "knowledge.sock");
    if socket.exists() {
        Ok(socket)
    } else {
        Err(NOT_RUNNING.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The marker has to survive being carried through a `String` error and back
    /// out to the frontend, which matches on it. A rename here without one there
    /// turns the honest sentence back into the misleading one, silently.
    #[test]
    fn the_marker_is_the_string_the_frontend_looks_for() {
        assert_eq!(NOT_RUNNING, "knowledge-daemon-not-running");
    }

    #[test]
    fn an_absent_socket_is_reported_as_absent_rather_than_as_a_path() {
        // Point the resolver at a directory with no socket in it.
        let dir = std::env::temp_dir().join("arlen-knowledge-absent-probe");
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("definitely-not-here.sock");
        std::env::set_var("ARLEN_KNOWLEDGE_SOCKET", &missing);
        assert_eq!(socket_or_absent().unwrap_err(), NOT_RUNNING);
        std::env::remove_var("ARLEN_KNOWLEDGE_SOCKET");
    }
}
