//! Behaviour discovery wiring: the agent's concrete search paths, its
//! config-file location, and the configured-load entry point.
//!
//! The discovery *mechanism* (the behaviour search paths, stamping provenance,
//! resolving enablement, fail-soft loading) lives in the shared
//! [`arlen_ai_skills::loader`]; this module supplies the agent-specific inputs
//! the shared crate must not depend on — where `ai.toml` is and how enablement
//! is read from the agent's [`AgentConfig`].

use std::path::PathBuf;

use arlen_ai_skills::loader::{behaviour_sources, load, LoadOutcome};

use crate::config::AgentConfig;

/// Path of the agent's TOML config: the `ARLEN_AI_CONFIG` override, else
/// `~/.config/arlen/ai.toml`.
pub fn ai_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("ARLEN_AI_CONFIG") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/arlen/ai.toml")
}

/// Load every discoverable behaviour and stamp its enablement from the agent's
/// current config, exactly as the running daemon does. A missing or unreadable
/// config fails closed (nothing enabled). This is the read-only entry point an
/// observer (the harness behaviour-status view) uses to show the same set the
/// daemon would act on, without duplicating the discovery + enablement wiring.
pub fn load_configured() -> LoadOutcome {
    let config = match std::fs::read_to_string(ai_config_path()) {
        Ok(text) => AgentConfig::parse(&text),
        Err(_) => AgentConfig::fail_closed(),
    };
    load(&behaviour_sources(), &config.enabled)
}
