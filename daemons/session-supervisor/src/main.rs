//! Placeholder entry point while the systemd and broker seams are wired.
//!
//! The decision half ([`arlen_session_supervisor::supervise`]) is built and
//! tested against a scripted systemd. This binary gains its D-Bus client and its
//! pidfd registration next; until then it exits rather than pretending to
//! supervise, because a supervisor that runs and registers nothing is exactly the
//! silent-success shape the rest of this work exists to remove.

fn main() -> std::process::ExitCode {
    eprintln!(
        "arlen-session-supervisor: the systemd and broker seams are not wired yet, \
         so nothing is supervised and no identity is registered"
    );
    std::process::ExitCode::FAILURE
}
