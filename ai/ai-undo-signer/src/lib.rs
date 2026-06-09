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
//! Built smallest-sound-sub-step first. This crate currently provides the **key
//! custody** ([`key`]) and the private state-directory resolution ([`paths`]).
//! The chained on-disk log (reusing `arlen-ai-agent::undo_log`'s chain scheme),
//! the peer-authed submit/lookup socket, and the access control are later
//! increments built on this.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod error;
pub mod key;
pub mod paths;

pub use error::{Result, SignerError};
