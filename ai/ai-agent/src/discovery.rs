//! Behaviour discovery wiring: the agent's concrete search paths, its
//! config-file location, and the configured-load entry point.
//!
//! The discovery *mechanism* (stamping provenance, resolving enablement,
//! fail-soft loading) lives in the shared [`arlen_ai_skills::loader`]; this
//! module supplies the agent-specific inputs — where behaviours are
//! installed, where `ai.toml` is, and how enablement is read from the
//! agent's [`AgentConfig`] — which the shared crate must not depend on.

use std::path::PathBuf;

use arlen_ai_skills::loader::{load, BehaviourSource, LoadOutcome};

use crate::config::AgentConfig;

/// The canonical behaviour-source search path in precedence order: the system
/// behaviours directory, then the user directory. In debug builds an
/// `ARLEN_AGENT_BEHAVIOURS` directory is appended as a stand-in for the
/// not-yet-installed system directory; it is compiled out of release builds so
/// an environment variable can never inject built-in-provenance behaviours into
/// a deployed system.
pub fn behaviour_sources() -> Vec<BehaviourSource> {
    let mut sources = vec![BehaviourSource::builtin("/usr/share/arlen/agent/behaviours")];
    if let Ok(home) = std::env::var("HOME") {
        sources.push(BehaviourSource::user(format!(
            "{home}/.local/share/arlen/agent/behaviours"
        )));
    }
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("ARLEN_AGENT_BEHAVIOURS") {
        sources.push(BehaviourSource::builtin(dir));
    }
    sources
}

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
