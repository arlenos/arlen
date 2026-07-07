//! The configured-load entry point: resolve the AI config location and load the
//! behaviours it enables. Lifted from the agent into the shared crate so any
//! consumer (the daemon, the harness) loads the enabled behaviours from `ai.toml`
//! without depending on the agent's full config type. The daemon's `AgentConfig`
//! builds its `enabled` map the same way (each `[agent] enabled` name is a
//! built-in behaviour), so this and the running daemon agree on what is enabled.

use crate::loader::{behaviour_sources, load, LoadOutcome, Provenance};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Path of the AI config: the `ARLEN_AI_CONFIG` override, else
/// `~/.config/arlen/ai.toml`.
pub fn ai_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("ARLEN_AI_CONFIG") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/arlen/ai.toml")
}

/// The `[agent]` section fields this loader reads (only the enabled list; the
/// daemon's `AgentConfig` reads the rest). `#[serde(default)]` so a config with no
/// `[agent]` section, or none listed, enables nothing.
#[derive(serde::Deserialize, Default)]
struct AgentSection {
    #[serde(default)]
    enabled: Vec<String>,
}

/// The subset of `ai.toml` this loader parses. Unknown keys are ignored, so it
/// tolerates the daemon's richer config.
#[derive(serde::Deserialize, Default)]
struct AiConfigFile {
    #[serde(default)]
    agent: AgentSection,
}

/// Load the behaviours enabled in the current `ai.toml`, exactly as the running
/// daemon does. A missing or unreadable config fails closed (nothing enabled).
pub fn load_configured() -> LoadOutcome {
    load(&behaviour_sources(), &read_enabled(&ai_config_path()))
}

/// Parse the `[agent] enabled` list from the config at `path` into the
/// enabled-behaviours map (each name a built-in behaviour), fail-closed to empty
/// on any read or parse error.
fn read_enabled(path: &Path) -> BTreeMap<String, Provenance> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let parsed: AiConfigFile = toml::from_str(&text).unwrap_or_default();
    parsed
        .agent
        .enabled
        .into_iter()
        .map(|name| (name, Provenance::BuiltIn))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_config(body: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(body.as_bytes()).unwrap();
        f
    }

    #[test]
    fn reads_the_enabled_list_as_builtin_behaviours() {
        let f = write_config("[agent]\nenabled = [\"auto-tag-by-project\", \"meeting-prep\"]\n");
        let enabled = read_enabled(f.path());
        assert_eq!(enabled.get("auto-tag-by-project"), Some(&Provenance::BuiltIn));
        assert_eq!(enabled.get("meeting-prep"), Some(&Provenance::BuiltIn));
        assert_eq!(enabled.len(), 2);
    }

    #[test]
    fn a_missing_config_enables_nothing() {
        assert!(read_enabled(Path::new("/definitely/no/such/ai.toml")).is_empty());
    }

    #[test]
    fn a_config_without_an_agent_section_enables_nothing() {
        let f = write_config("[ai]\nenabled = true\n");
        assert!(read_enabled(f.path()).is_empty());
    }

    #[test]
    fn malformed_toml_fails_closed_to_empty() {
        let f = write_config("this is not = valid = toml [[[");
        assert!(read_enabled(f.path()).is_empty());
    }
}
