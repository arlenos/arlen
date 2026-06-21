//! Arlen foreign-app bridge ingestion: the core (built once, shared by all
//! bridges) that turns an untrusted foreign plugin's messages into KG entity
//! upserts under a declarative, per-bridge `bridge.toml` mapping
//! (foreign-app-bridges.md). A bridge ships no code here — only declarative
//! data (`entities.toml` + `bridge.toml`); this crate is the privileged side.
//!
//! Built so far: the `bridge.toml` schema + validation ([`bridge`]), the pure
//! message -> upsert-plan interpreter ([`interpret`]), and the native-messaging
//! stdio host ([`host`]: length-prefixed framing + the mutual-id-pin handshake +
//! untrusted-message validation, routing each ingest to a [`host::PlanSink`]).
//! The remaining slice is the real sink: writing a plan through the bridge's
//! macaroon-scoped, origin-tagged app-tier entity-write socket - gated on the
//! macaroon namespace-delegation in the knowledge write path (see the daemon's
//! coder report; a bridge writes a namespace that is not its own app id).

pub mod bridge;
pub mod host;
pub mod interpret;
pub mod sink;

pub use bridge::{BridgeConfig, BridgeError, BridgeMeta, LinkRule, MapRule};
pub use host::{serve, HostError, InboundMessage, OutboundMessage, PlanSink, MAX_FRAME};
pub use interpret::{interpret_message, InterpretError, LinkPlan, UpsertPlan};
pub use sink::{EntityWriter, KgPlanSink};
