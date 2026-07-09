//! Shared undo-log vocabulary for the reversible-receipts effect model
//! (reversible-receipts-and-the-effect-model.md §5, §6).
//!
//! A submitter captures inverse receipts and proposes lifecycle transitions; the
//! separate-uid signer (`arlen-ai-undo-signer`) seals and chains them. Both sides
//! need the same record vocabulary and the same chain scheme, so they live here
//! rather than in either binary's crate. The original submitter (the native
//! `arlen-ai-agent`) is retired; this vocabulary is the substrate a future pi-side
//! persisted-undo re-home builds on (pi's in-memory compensation is the interim).
//!
//! [`effect_model`] is the pure inverse + classification vocabulary; [`undo_log`]
//! is the event-sourced lifecycle state machine, the in-memory store, and the
//! HMAC-chained `FileUndoLog`.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod effect_model;
pub mod undo_log;
