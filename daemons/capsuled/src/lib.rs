//! Context Capsule daemon (context-capsule.md): lend a part of your Knowledge
//! Graph outward under control — a signed, scope-bound, frozen, expiring,
//! op-count-limited, revocable, audited slice, minted and revoked through the
//! same capability + audit machinery as an in-process reach.
//!
//! Day-one scope is the **same-machine capsule**: full mint/serve/audit/revoke on
//! the box that holds the graph, provable end-to-end. Cross-machine sync is gated
//! on an undesigned device-pairing + transport substrate (CC-R8), and the
//! external-agent grant is behind a human-gated flag (CC-R7); neither is built
//! here.
//!
//! The capsule wire contract — the scope selector and the frozen-slice model with
//! its canonical serialization — lives in the shared [`arlen_capsule`] crate, so
//! the knowledge daemon (which materializes a slice as of `T_mint`), this daemon,
//! and the os-sdk client share one definition. This crate adds the capsuled-side
//! pieces: [`store`] content-addresses a materialized slice in the forage store
//! (the frozen, refcounted blob); the grant, the serve loop and the mint/revoke
//! surface follow.

pub use arlen_capsule::{scope, slice};

pub mod grant;
pub mod key;
pub mod proto;
pub mod revocation;
pub mod serve;
pub mod store;
