//! Not yet the session. See `lib.rs` for why this crate exists and what is built.
//!
//! The decisions are ported and tested; the process work - spawning the
//! compositor, waiting for its socket, importing the environment, starting the
//! shell - is not. Until it is, the shipped session stays the shell script, and
//! this refuses to run rather than half-starting a login.

fn main() -> std::process::ExitCode {
    eprintln!(
        "arlen-session: the compiled session is not complete - the shipped \
         /usr/bin/arlen-session script still owns the login path"
    );
    std::process::ExitCode::FAILURE
}
