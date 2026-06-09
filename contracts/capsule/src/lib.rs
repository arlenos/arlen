//! Context Capsule wire contract (context-capsule.md).
//!
//! The shared vocabulary of a capsule mint: the instance-set [`scope`] selector
//! and the frozen-slice [`slice`] model with its canonical serialization. These
//! are the request and response of the capsule read: the knowledge daemon owns
//! the graph and the bitemporal as-of read, so it materializes the [`slice`] for a
//! [`scope::CapsuleScope`] as of `T_mint` (the BFS expansion runs server-side with
//! a graph-backed neighbour source); the `capsuled` daemon sends the scope,
//! receives the frozen slice, content-addresses it and signs the grant; the
//! os-sdk client carries both. One definition, shared, so the request shape and
//! the canonical slice form cannot drift between the daemon and its callers.

pub mod scope;
pub mod slice;
