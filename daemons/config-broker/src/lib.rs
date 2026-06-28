//! Config Broker - the separate-uid owner of the AI master switches.
//!
//! Today the AI's security-load-bearing settings (`enabled`,
//! `access_level`, `executor_live`, `provider`, `action_mode`,
//! `autonomous_apps`) live in `~/.config/arlen/ai.toml`, a plain
//! user-owned file any same-uid process can rewrite - and
//! `executor_live`'s "human gate" IS that boolean, so flipping the
//! file flips the gate. `same-uid-isolation-plan.md` Tier-A #1: a
//! daemon running as a SEPARATE uid owns the canonical state in a
//! directory the user's normal uid cannot write, and mutates it only
//! over a `SO_PEERPIDFD`-authenticated socket (the auth primitive is
//! `arlen_permissions::peer_pidfd`).
//!
//! This crate is built in slices: [`state`] is the canonical store
//! (the typed master switches + its 0700-dir / 0600-file durable
//! read-write); the socket + setter protocol + the admitted-caller
//! gate land on top of it.

pub mod state;

pub use state::{ActionMode, AiMasterSwitches, StateError, StateStore};
