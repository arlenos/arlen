//! The Arlen graphical session, as decisions rather than as a shell script.
//!
//! `arlen-session` is what greetd's `[initial_session]` starts: it mints the
//! session id, settles the environment, brings up the compositor and the shell,
//! and decides what else the session runs. It has been a `/bin/sh` script since it
//! was written, and two independent reasons now say it should not be.
//!
//! **It cannot be identified.** A running script's `/proc/<pid>/exe` is the
//! INTERPRETER - measured 13 Aug, `/usr/bin/bash` for a `#!/bin/sh` script - so
//! every script on the machine presents the same identity. `comm` carries the
//! script's name but a process sets its own with `prctl(PR_SET_NAME)`, so it
//! attests nothing. A session root that cannot be told apart from any other script
//! cannot anchor which processes may stamp an identity, which is what the
//! registrar model needs of it.
//!
//! **Whoever can write the file becomes the session root.** A script is read at
//! start from a path; a binary is a file whose inode the identity registry can
//! attest. That difference is the whole trust argument.
//!
//! Independently of both: it mints the session id, which everything downstream
//! joins on, and it decides what the session starts. Those are authority-adjacent
//! jobs, and a shell script is the wrong artefact for them.
//!
//! This crate holds the DECISIONS - the parts that can be tested without a
//! compositor, a seat or a login - so the port was verifiable in pieces rather than
//! only by booting. The process work (spawning, waiting, importing the environment,
//! stamping what it starts) sits in the binary on top of them.

pub mod env;
pub mod session_id;
pub mod stamp;
pub mod verify_app;
pub mod wayland;
