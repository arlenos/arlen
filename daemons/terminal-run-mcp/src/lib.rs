//! The Terminal-run MCP: run a single user-approved command inside the confiner's
//! sandbox (ai-act-layer-plan.md, the `run_command` sharp edge).
//!
//! `run_command` is the most powerful and most dangerous act: opaque, unboundable,
//! un-undoable (`OpaqueCommand` irreversibility). The design's whole safety story is
//! **always-Confirm + confined + output-captured + never-autonomous**, held by the
//! gate registry (which classifies `run_command` `Confirm`, never `Allow`). This
//! crate owns the CONFINED + OUTPUT-CAPTURED half: [`run::run_confined`] spawns the
//! command in the confiner's [`arlen_confiner::command_profile`] sandbox (no host
//! write, no network under `None`, no privilege), captures bounded stdout+stderr,
//! and enforces a wall-clock timeout. The always-Confirm gating is upstream (the pi
//! gate + the consent broker); this crate never decides to run - it runs what the
//! gate already approved.

pub mod run;
