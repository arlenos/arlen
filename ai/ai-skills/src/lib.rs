//! Shared skill model for the Arlen AI layer.
//!
//! The Agent-Skills (`SKILL.md`) parser, the event router, the
//! discovery + enablement loader, and the `whenToUse` skill matcher live
//! here so both `ai-agent` (the autonomous, event-triggered loop that also
//! *executes* skills) and `ai-daemon` (the interactive query daemon that
//! *matches* a free-form task to a skill before its plain-answer fallback)
//! depend on one definition. Skill **execution** stays in `ai-agent`; this
//! crate is the static model + discovery only, with no execution, graph, or
//! provider coupling.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod behaviour;
pub mod loader;
pub mod router;
pub mod skills;
