//! The AI-providers manager's daemon-owned enable/disable state (the manager
//! seam's mutable on/off, distinct from the Settings-owned `ai.toml` defaults).
//!
//! It is a small JSON set of DISABLED provider ids under the daemon's state dir;
//! a provider absent from the set is enabled. The daemon owns, persists, and
//! honours it, so there is no `ai.toml` co-ownership with Settings: `ai.toml`
//! carries the user's default provider/model + ranking (Settings-written), this
//! file carries which catalogued providers the manager turned off. Read by
//! `ai_providers_list` (to fill `enabled`), written by `ai_provider_set_enabled`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The disabled-providers state file:
/// `$XDG_STATE_HOME|$HOME/.local/state → arlen/ai-daemon/disabled-providers.json`.
/// `None` if neither base is set (then the manager is effectively read-only,
/// every provider enabled - fail-safe, a disable just does not persist).
pub fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(base.join("arlen/ai-daemon/disabled-providers.json"))
}

/// Load the set of disabled provider ids. Empty on an absent or unreadable or
/// corrupt file - fail-safe to all-enabled, never erroring the list.
pub fn load_disabled(path: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<BTreeSet<String>>(&text).ok())
        .unwrap_or_default()
}

/// Persist `enabled` for `id`: remove it from the disabled set when enabling,
/// add it when disabling. Read-modify-write the JSON set atomically (a sibling
/// temp file + rename). Idempotent - enabling an already-enabled provider (or
/// disabling an already-disabled one) is a no-op that still rewrites the same
/// set. Creates the parent dir on first use.
pub fn set_enabled(path: &Path, id: &str, enabled: bool) -> std::io::Result<()> {
    let mut disabled = load_disabled(path);
    let changed = if enabled {
        disabled.remove(id)
    } else {
        disabled.insert(id.to_string())
    };
    // Even a no-op write keeps the file present + canonical; cheap and simpler
    // than branching on `changed`.
    let _ = changed;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string(&disabled)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Whether `id` is enabled given the current disabled set on disk (the helper
/// `ai_providers_list` uses per provider).
pub fn is_enabled(path: &Path, id: &str) -> bool {
    !load_disabled(path).contains(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_disable_round_trips_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("arlen/ai-daemon/disabled-providers.json");

        // Absent file: everything enabled.
        assert!(load_disabled(&path).is_empty());
        assert!(is_enabled(&path, "anthropic"));

        // Disable one provider; only it is disabled.
        set_enabled(&path, "anthropic", false).unwrap();
        assert_eq!(load_disabled(&path).iter().cloned().collect::<Vec<_>>(), vec!["anthropic"]);
        assert!(!is_enabled(&path, "anthropic"));
        assert!(is_enabled(&path, "ollama-default"));

        // Disabling again is a no-op (still exactly one entry).
        set_enabled(&path, "anthropic", false).unwrap();
        assert_eq!(load_disabled(&path).len(), 1);

        // Re-enable removes it.
        set_enabled(&path, "anthropic", true).unwrap();
        assert!(load_disabled(&path).is_empty());
        assert!(is_enabled(&path, "anthropic"));
    }

    #[test]
    fn a_corrupt_state_file_fails_safe_to_all_enabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disabled.json");
        std::fs::write(&path, b"not json").unwrap();
        assert!(load_disabled(&path).is_empty(), "corrupt -> all enabled, never errors");
    }
}
