//! Arlen foreign-app bridge ingestion: the core (built once, shared by all
//! bridges) that turns an untrusted foreign plugin's messages into KG entity
//! upserts under a declarative, per-bridge `bridge.toml` mapping
//! (foreign-app-bridges.md). A bridge ships no code here — only declarative
//! data (`entities.toml` + `bridge.toml`); this crate is the privileged side.
//!
//! Built so far: the `bridge.toml` schema + validation ([`bridge`]) and the
//! pure message -> upsert-plan interpreter ([`interpret`]). The native-
//! messaging stdio host (mutual id-pin transport) and the write of a plan
//! through the macaroon-scoped app-tier entity-write socket are the next
//! slices (see the daemon's coder report for the macaroon namespace-
//! delegation dependency the shared-daemon write half rests on).

pub mod bridge;
pub mod interpret;

pub use bridge::{BridgeConfig, BridgeError, BridgeMeta, LinkRule, MapRule};
pub use interpret::{interpret_message, InterpretError, LinkPlan, UpsertPlan};
