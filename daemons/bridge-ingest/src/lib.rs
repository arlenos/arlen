//! Arlen foreign-app bridge ingestion: the core (built once, shared by all
//! bridges) that turns an untrusted foreign plugin's messages into KG entity
//! upserts under a declarative, per-bridge `bridge.toml` mapping
//! (foreign-app-bridges.md). A bridge ships no code here — only declarative
//! data (`entities.toml` + `bridge.toml`); this crate is the privileged side.
//!
//! Built: the `bridge.toml` schema + validation ([`bridge`]), the pure message ->
//! upsert-plan interpreter ([`interpret`]), the native-messaging stdio host
//! ([`host`]: length-prefixed framing + the mutual-id-pin handshake +
//! untrusted-message validation, routing each ingest to a [`host::PlanSink`]), the
//! Obsidian vault floor reader ([`obsidian`] + the file-watch [`watch`]), and the
//! real KG sink ([`sink::KgPlanSink`] over an [`sink::EntityWriter`]; the daemon
//! binary's `GraphEntityWriter` drives the os-sdk `upsert_entity`/`link_entities`
//! clients).
//!
//! The sink writes through the bridge's origin-tagged app-tier entity-write socket.
//! The namespace-delegation it relies on - a bridge writing entity types under a
//! namespace that is not its own app id (e.g. `md.obsidian.*`) - is now enforced in
//! the knowledge write path (`daemons/knowledge/src/write/namespace_grant.rs` +
//! `plan_entity_upsert`/`plan_entity_link`): the bridge's profile declares
//! `[graph].delegated_namespaces`, validated reserved-deny + attenuate-only, with
//! `system.*`/`shared.*` structurally unwritable. So the ingestion path is complete
//! end to end. What remains is DEPLOYMENT, not code: provisioning a per-bridge
//! permission profile that declares its delegated namespace + write scope, which
//! rides the forage auto-install grant flow (foreign-app-bridges.md item 3, gated
//! on forage's install path).

pub mod bridge;
pub mod host;
pub mod interpret;
pub mod obsidian;
pub mod sink;
pub mod watch;

pub use bridge::{BridgeConfig, BridgeError, BridgeMeta, LinkRule, MapRule};
pub use host::{serve, HostError, InboundMessage, OutboundMessage, PlanSink, MAX_FRAME};
pub use interpret::{interpret_message, InterpretError, LinkPlan, UpsertPlan};
pub use sink::{EntityWriter, KgPlanSink};
pub use watch::{ingest_note, sync_vault, watch_vault};
