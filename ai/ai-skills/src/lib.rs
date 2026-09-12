//! Shared skill model for the Arlen AI layer.
//!
//! The Agent-Skills (`SKILL.md`) parser, the event router, the discovery +
//! enablement loader, and the `whenToUse` skill matcher live here so every
//! reader of a skill works off one definition: `ai-engine-daemon` loads and runs
//! them, the harness and Settings list them, and the router decides which event
//! reaches which. Skill **execution** stays in the engine; this crate is the
//! static model + discovery only, with no execution, graph, or provider
//! coupling.
//!
//! The matcher in [`skills`] is the one part with no reader yet, and its own
//! module says why: routing a question to a matched skill is a read-scope
//! decision rather than a wiring job.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod behaviour;
pub mod loader;
pub mod router;
pub mod skills;

/// The configured-load entry point (read ai.toml, load its enabled behaviours).
pub mod discovery;
