//! The Arlen AI engine daemon: supervises a swappable agent engine (pi) behind
//! the engine-neutral five-verb contract, authenticates the engine process via
//! SO_PEERCRED + a daemon-minted session token, and bounds every action to the
//! session grant server-side. See `pi-agent-adoption.md`.
//!
//! This is built BESIDE the existing ai-daemon/ai-agent (two systems side by
//! side); nothing here touches them. Phase 0 lands the session-auth core; the
//! contract socket, verb dispatch, and pi-sidecar supervision follow.

pub mod session;

pub mod dispatch;

pub mod wire;
