//! The Arlen Connections daemon (`org.arlen.Connections1`): the single,
//! capability-gated credential authority (connections-plan.md).
//!
//! It owns all integration credentials (OAuth refresh tokens, API keys, webhook
//! secrets) under a TPM/PCR-sealed master key, and hands out only per-app-scoped,
//! downscoped tokens, never the raw stored credential and never the whole
//! keyring. The account-daemon folds in under it as one credential class.
//!
//! This crate is being built incrementally (CONN-R1 first). The foundation here
//! is the pure [`broker`] authorization core: the powerbox decision that gates
//! every handout on a standing capability grant with strict monotonic
//! attenuation. The credential store (master-key-sealed, reusing the reviewed
//! AEAD vault pattern), the daemon socket, and the peer-auth layer build on top.

pub mod broker;
pub mod config;
pub mod store;
