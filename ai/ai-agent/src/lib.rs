//! Agent daemon library for the Arlen AI layer.
//!
//! Hosts the D-Bus interface (`org.arlen.AIAgent1`), the Event Bus
//! subscriber, and the per-behaviour trigger dispatcher. Disabling the
//! last enabled behaviour stops the binary entirely so an inactive
//! agent layer has no running process.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod agentic;
mod canary;
pub mod dbus;
pub mod compaction;
pub mod config;
pub mod discovery;
pub mod engine;
pub mod executor;
pub mod fs_move;
pub mod gate;
pub mod graph;
pub mod handlers;
pub mod receipt_store;
mod registry;
pub mod seams;
pub mod slice;
pub mod snapshot;
pub mod source;
pub mod spill;
pub mod undo_client;
pub mod working_set;
pub mod world;

// The undo-log vocabulary (the inverse-receipt effect model, the event-sourced
// lifecycle, the HMAC-chained log) is shared with the separate-uid signer
// helper, so it lives in `arlen-ai-undo-core` and is re-exported here. Internal
// references stay `crate::effect_model` / `crate::undo_log`.
pub use arlen_ai_undo_core::{effect_model, undo_log};

// The static skill model (the SKILL.md parser, the event router, the
// discovery+enablement loader, the whenToUse matcher) is shared with the
// interactive `ai-daemon` agent-match path, so it lives in `arlen-ai-skills`
// and is re-exported here. Internal references stay `crate::behaviour` /
// `crate::router` / `crate::loader` / `crate::skills`; the agent-specific
// discovery wiring (search paths + config load) is `crate::discovery`.
pub use arlen_ai_skills::{behaviour, loader, router, skills};
