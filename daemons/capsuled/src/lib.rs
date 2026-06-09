//! Context Capsule (context-capsule.md): lend a part of your Knowledge Graph
//! outward under control — a signed, scope-bound, frozen, expiring,
//! op-count-limited, revocable, audited slice, minted and revoked through the
//! same capability + audit machinery as an in-process reach.
//!
//! Day-one scope is the **same-machine capsule**: full mint/serve/audit/revoke on
//! the box that holds the graph, provable end-to-end. Cross-machine sync is gated
//! on an undesigned device-pairing + transport substrate (CC-R8), and the
//! external-agent grant is behind a human-gated flag (CC-R7); neither is built
//! here.
//!
//! This crate is being built bottom-up. The first piece is [`scope`], the net-new
//! instance-set scope selection the canonical "share this project" example needs
//! (the existing `InstanceScope` is only `Own | All`, with no "exactly these node
//! ids" form). The frozen-slice materializer, the signed grant, the `capsuled`
//! serve loop and the mint/revoke surface follow.

pub mod scope;
pub mod slice;
pub mod store;
