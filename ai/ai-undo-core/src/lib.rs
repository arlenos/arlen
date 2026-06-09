//! Shared undo-log vocabulary for the reversible-receipts effect model
//! (reversible-receipts-and-the-effect-model.md §5, §6).
//!
//! The agent (the submitter, `arlen-ai-agent`) captures inverse receipts and
//! proposes lifecycle transitions; the separate-uid signer (`arlen-ai-undo-signer`)
//! seals and chains them. Both sides need the same record vocabulary and the same
//! chain scheme, so they live here rather than in either binary's crate.
//!
//! [`effect_model`] is the pure inverse + classification vocabulary; [`undo_log`]
//! is the event-sourced lifecycle state machine, the in-memory store, and the
//! HMAC-chained `FileUndoLog`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod effect_model;
pub mod undo_log;
