//! Config Broker - the separate-uid owner of the user config the session reads.
//!
//! Two families live here now, and the name is deliberately about neither of
//! them. What decides whether a setting belongs in this daemon is not its
//! subject but its threat: a value that a same-uid process must not be able to
//! rewrite, because something downstream treats it as authority. The AI master
//! switches were the first. Accessibility is the second, for the opposite
//! reason - not that changing it grants power, but that LOSING it costs a
//! person the use of their machine, so it must not ride along with taste. A
//! theme switch must never turn a screen reader off.
//!
//! ON THE NAME: the crate, socket, unit and state directory are already
//! scope-neutral (`arlen-config-broker`, `config-broker.sock`,
//! `arlen-config-broker.service`, `ARLEN_CONFIG_BROKER_DIR`) - nothing a
//! reader meets from outside says AI. What said AI was inside: the type was
//! `AiMasterSwitches` and the docs described the whole daemon as its owner.
//! So the rename that was actually owed was of the state type, which is the
//! generalisation rather than a step after it, and renaming the external
//! surface would have churned the socket table, the identity tables and the
//! shipped-unit checks to arrive at a name it already had.
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
//! [`state`] is the canonical store ([`BrokerState`], one field per family,
//! with a 0700-dir / 0600-file durable read-write); [`protocol`] carries the
//! per-family set ops and the admitted-caller gate; [`server`] serves them
//! over the authenticated socket and audits the escalating ones.
//!
//! A writer that holds one family must go through that family's setter
//! ([`StateStore::store_ai`], [`StateStore::store_accessibility`]). The
//! families share one file, so writing the whole state from one family's
//! value would write the others back as defaults, which is not a default,
//! it is an erase - the shape `dev/scripts/check-default-then-write.py`
//! exists to catch, and this crate should not be its first finding.

pub mod client;
pub mod identity_op;
pub mod identity_server;
pub mod protocol;
pub mod server;
pub mod state;

pub use client::{ClientError, ConfigBrokerClient};
pub use protocol::{handle_request, is_admitted_writer, Request, Response};
pub use state::{
    Accessibility, ActionMode, AiMasterSwitches, BrokerState, StateError, StateStore,
};
