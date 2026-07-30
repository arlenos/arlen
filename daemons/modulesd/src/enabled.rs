//! The user's disabled-module list, as modulesd sees it.
//!
//! There were two disjoint answers to "is this module enabled". Settings and
//! the shell's loader read and write `~/.config/arlen/modules.toml` (a flat
//! list of disabled ids); modulesd held its own in-memory flag, defaulted
//! `true` at discovery and mutated only by the `SetEnabled` socket request,
//! and never read that file. So a module the user switched off in Settings
//! stayed enabled inside the runtime that enforces its capabilities: it kept
//! minting iframe nonces, kept serving its MCP tools to the AI, and kept
//! passing capability checks on host calls.
//!
//! This reads the same file the UI writes, so discovery starts from the user's
//! actual intent instead of admitting everything. It only ever DISABLES - a
//! module named here is off, and an unreadable or absent file means no module
//! is disabled, which is the file's own meaning rather than a fallback.

use std::collections::BTreeSet;
use std::path::PathBuf;

use serde::Deserialize;

/// `~/.config/arlen/modules.toml`, matching what Settings and the shell loader
/// use. `XDG_CONFIG_HOME` wins, else `$HOME/.config`.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("arlen").join("modules.toml"))
}

/// The file's shape: `[disabled] modules = ["com.example.thing"]`.
#[derive(Debug, Default, Deserialize)]
struct ModulesConfig {
    #[serde(default)]
    disabled: DisabledSection,
}

#[derive(Debug, Default, Deserialize)]
struct DisabledSection {
    #[serde(default)]
    modules: Vec<String>,
}

/// Parse a disabled list from the file's text.
///
/// Malformed TOML yields an EMPTY set, not an error. That is deliberate and is
/// the safe direction here: this list only ever turns modules off, so failing
/// to read it leaves every module in the state it would have had before, and
/// the consent gate on `SetEnabled` still stands. Refusing to start, or
/// disabling everything, would take the whole extension surface down over a
/// stray character in a config file.
pub fn parse_disabled(text: &str) -> BTreeSet<String> {
    toml::from_str::<ModulesConfig>(text)
        .map(|c| c.disabled.modules.into_iter().collect())
        .unwrap_or_default()
}

/// Read the user's disabled list. An absent file means nothing is disabled,
/// which is what an absent file means to Settings too.
pub fn disabled_modules() -> BTreeSet<String> {
    config_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse_disabled(&t))
        .unwrap_or_default()
}

/// Render a disabled list back to the file's text, sorted so a rewrite does
/// not reshuffle the file and produce a spurious diff.
pub fn render_disabled(disabled: &BTreeSet<String>) -> String {
    let mut out = String::from("[disabled]\nmodules = [");
    for (i, id) in disabled.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("\n  {id:?}"));
    }
    if !disabled.is_empty() {
        out.push('\n');
    }
    out.push_str("]\n");
    out
}

/// Record `module_id` as enabled or disabled in the user's list.
///
/// Read-modify-write against the same file Settings edits, so the two agree
/// after a socket-driven toggle instead of the runtime silently reverting on
/// restart. Best-effort: a write failure is reported to the caller to log, not
/// to fail the toggle on, because the in-memory state is what governs this run
/// and refusing would leave the user unable to switch a module off at all.
pub fn persist(module_id: &str, enabled: bool) -> Result<(), String> {
    let path = config_path().ok_or_else(|| "no config directory".to_string())?;
    let mut disabled = std::fs::read_to_string(&path)
        .map(|t| parse_disabled(&t))
        .unwrap_or_default();
    let changed = if enabled {
        disabled.remove(module_id)
    } else {
        disabled.insert(module_id.to_string())
    };
    if !changed {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    std::fs::write(&path, render_disabled(&disabled))
        .map_err(|e| format!("write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_disabled_module_is_read_back() {
        let set = parse_disabled("[disabled]\nmodules = [\"com.example.a\", \"com.example.b\"]\n");
        assert!(set.contains("com.example.a"));
        assert!(set.contains("com.example.b"));
        assert_eq!(set.len(), 2);
    }

    /// An absent section is the common case on a fresh machine and must not
    /// read as "everything disabled".
    #[test]
    fn an_empty_or_absent_list_disables_nothing() {
        assert!(parse_disabled("").is_empty());
        assert!(parse_disabled("[disabled]\n").is_empty());
        assert!(parse_disabled("[something_else]\nx = 1\n").is_empty());
    }

    /// What modulesd writes has to be what Settings reads back, or a toggle in
    /// one surface is invisible in the other.
    #[test]
    fn a_rendered_list_parses_back_to_itself() {
        let mut set = BTreeSet::new();
        set.insert("com.example.b".to_string());
        set.insert("com.example.a".to_string());
        assert_eq!(parse_disabled(&render_disabled(&set)), set);
        assert_eq!(parse_disabled(&render_disabled(&BTreeSet::new())), BTreeSet::new());
    }

    /// A stray character in a config file must not take the whole extension
    /// surface down, in either direction.
    #[test]
    fn malformed_toml_disables_nothing_rather_than_everything() {
        assert!(parse_disabled("[disabled]\nmodules = not-a-list").is_empty());
        assert!(parse_disabled("{{{").is_empty());
    }
}
