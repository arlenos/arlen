//! The separate-uid signer helper for the AI agent's durable undo-log
//! (reversible-receipts-and-the-effect-model.md EM-R1, §6).
//!
//! The undo-log holds exactly the PII-adjacent prior state (the prior path, the
//! prior value) the agent needs to reverse a non-graph action. Integrity and
//! confidentiality *against the agent itself* (the F3 same-uid concern) require
//! this store to be owned by a different uid: the agent submits entries over a
//! socket, and this helper seals them under an HMAC key the agent never holds and
//! serves undo lookups back under the same gate. It is the same architectural
//! move as the root-owned `permission-helper` and the `audit-daemon`'s own key
//! custody.
//!
//! Built smallest-sound-sub-step first. This crate provides the **key custody**
//! ([`key`]), the private state-directory resolution ([`paths`]), and the
//! signer's **local sealed store** ([`store::SignerStore`]: the HMAC-chained
//! `arlen-ai-undo-core` log opened with the custodied key, fail-closed on a
//! missing key or a broken chain). The peer-authed submit/lookup socket and the
//! access control that front this store for the agent are later increments.

// `deny` rather than `forbid`: the one unavoidable `unsafe` is `libc::getuid()`
// (no safe std wrapper), audited with a single `#[allow]` as the audit-daemon
// does. Every other use of unsafe stays denied.
#![deny(unsafe_code)]
#![warn(missing_docs)]

pub mod auth;
pub mod error;
pub mod key;
pub mod paths;
pub mod store;

pub use error::{Result, SignerError};
pub use store::SignerStore;
