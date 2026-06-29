//! The file manager's Ask-Arlen off-switch read (file-manager-plan.md item 6b).
//!
//! `files_ai_enabled` mirrors the harness `ai_capability` enabled bit: it reads
//! `[ai].enabled` from `~/.config/arlen/ai.toml` — the same file the ai-daemon
//! resolves its read scope from — so the Ask-mode affordance shows up only when
//! the AI layer the daemon enforces is actually on. Read-only and advisory: a
//! missing or malformed config reads as `false` (Ask unavailable), the safe
//! fail-closed default matching the daemon's own posture.

/// Whether the `[ai].enabled` flag is set in the parsed config. Absent,
/// non-boolean, or a missing `[ai]` table all read as `false` — the fail-closed
/// default, so a config the daemon would treat as disabled never shows the
/// Ask affordance. Pure, so it is unit-tested without a config file.
fn is_ai_enabled(doc: &toml::Table) -> bool {
    doc.get("ai")
        .and_then(toml::Value::as_table)
        .and_then(|t| t.get("enabled"))
        .and_then(toml::Value::as_bool)
        .unwrap_or(false)
}

/// Resolve `~/.config/arlen/ai.toml` (via `dirs::config_dir`), falling back to a
/// bare relative path if no config dir is resolvable (then the read simply misses
/// and the gate reads disabled).
fn ai_config_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_default()
        .join("arlen")
        .join("ai.toml")
}

/// Whether the Ask-Arlen mode is available: the `[ai].enabled` flag from
/// `ai.toml`. Never errors — an unreadable or malformed config reads as `false`
/// (Ask unavailable), so the off-switch is fail-closed.
#[tauri::command]
pub fn files_ai_enabled() -> bool {
    let Ok(text) = std::fs::read_to_string(ai_config_path()) else {
        return false;
    };
    let Ok(doc) = text.parse::<toml::Table>() else {
        return false;
    };
    is_ai_enabled(&doc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_true_only_when_the_flag_is_set() {
        let on = "[ai]\nenabled = true\n".parse::<toml::Table>().unwrap();
        assert!(is_ai_enabled(&on));
        let off = "[ai]\nenabled = false\n".parse::<toml::Table>().unwrap();
        assert!(!is_ai_enabled(&off));
    }

    #[test]
    fn missing_table_key_or_wrong_type_reads_disabled() {
        // Missing [ai], missing key, and a non-bool all fail closed.
        let no_table = "[provider]\nmodel = \"x\"\n".parse::<toml::Table>().unwrap();
        assert!(!is_ai_enabled(&no_table));
        let no_key = "[ai]\naccess_level = 2\n".parse::<toml::Table>().unwrap();
        assert!(!is_ai_enabled(&no_key));
        let wrong_type = "[ai]\nenabled = \"yes\"\n".parse::<toml::Table>().unwrap();
        assert!(!is_ai_enabled(&wrong_type));
    }
}
