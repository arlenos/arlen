//! Trash-first `rm` (CAH-2): the pure decision logic for the Arlen-native delete.
//!
//! `compensable-action-history-plan.md` §4 (Decision 2, LOCKED): ship a trash-first
//! delete that accepts `rm`'s flags and semantics but moves entries to the freedesktop
//! trash instead of unlinking, NEVER a naive `alias rm=trash` (aliasing breaks scripts
//! and muscle memory - flags, recursion, exit codes, `-f` all differ). This crate is
//! the pure core the binary is built on: the `rm`-flag parser and the trash-vs-unlink
//! ROUTING, both side-effect-free so the semantics are tested without touching a
//! filesystem. The trash-put + the RestoreFromTrash journal are the executing shell.
//!
//! Routing intent (the coder's call, per §4): an INTERACTIVE delete is trash-first and
//! reversible; a scripted/POSIX `rm` must not silently change semantics that break
//! tooling, so it stays a hard unlink. Distinguished at run time by whether stdout is
//! a tty and by an explicit purge flag.

pub mod parse;
pub mod route;
pub mod unlink;
