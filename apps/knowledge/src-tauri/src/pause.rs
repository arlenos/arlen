//! The timeline's Pause switch, written where the daemon reads it.
//!
//! The knowledge daemon enforces `[timeline] paused` in `graph.toml` at the point
//! events enter the store, and re-reads the file while it runs, so writing the key
//! here is the whole live path. This command exists because the app cannot call
//! Settings' `config_set`: a Tauri command is only reachable inside the binary
//! that registers it, which is the same trap `topbar_items` is stuck in.
//!
//! Format-preserving through `arlen-config-format` rather than a third TOML
//! writer of its own. `graph.toml` is a file people edit by hand - the watch
//! directories and the promotion threshold live in it, with comments explaining
//! them - and a pause that silently reformatted someone's config would be a poor
//! trade for one boolean.

use arlen_config_format::{handler_for, ConfigValue, Format};
use std::io::Write;
use std::path::PathBuf;

/// `~/.config/arlen/graph.toml`, the file the daemon reads.
fn graph_toml() -> Result<PathBuf, String> {
    dirs::config_dir()
        .map(|p| p.join("arlen/graph.toml"))
        .ok_or_else(|| "no config directory to write the setting into".to_string())
}

/// Pause or resume recording, and report the failure rather than swallowing it.
///
/// The app's own copy promises "Nothing is added until you resume", so a write
/// that did not land must surface: the caller puts the switch back and says
/// recording is still running. Returning `Ok` on a failed write is precisely the
/// lie this pair of surfaces was built to avoid.
#[tauri::command]
pub async fn knowledge_timeline_pause(paused: bool) -> Result<(), String> {
    let path = graph_toml()?;
    // A missing file is normal on a fresh install: the daemon defaults every
    // section, so the pause key is the first thing in it.
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };

    let handler = handler_for(Format::Toml);
    let updated = handler
        .set(&existing, "timeline.paused", &ConfigValue::Bool(paused))
        .map_err(|e| format!("{}: {e}", path.display()))?;

    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Temp file plus rename, like every other config write in the tree: a crash
    // mid-write must not leave a half-parsed graph.toml, which the daemon would
    // read as "no sections at all" and quietly resume recording.
    let tmp = path.with_extension("toml.tmp");
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(updated.as_bytes())
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.sync_all().map_err(|e| format!("{}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Is recording paused right now, according to the file the daemon reads?
///
/// The switch needs this on load. Persisting the pause means it survives a
/// restart, and a surface that opens showing "recording" while the daemon is
/// paused is the same lie as the one this pair was built to remove, pointing the
/// other way - the user would think they are being recorded when they are not,
/// and turn off something that is already off.
///
/// A missing or unreadable file answers `false`, matching the daemon's own
/// default: it records unless told otherwise, and the surface should say what
/// the daemon does rather than guess more cautiously than the truth.
#[tauri::command]
pub async fn knowledge_timeline_paused() -> Result<bool, String> {
    let Ok(path) = graph_toml() else { return Ok(false) };
    let Ok(text) = std::fs::read_to_string(&path) else { return Ok(false) };
    let model = handler_for(Format::Toml)
        .read(&text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(matches!(model.get("timeline.paused"), Some(ConfigValue::Bool(true))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(text: &str, paused: bool) -> String {
        handler_for(Format::Toml)
            .set(text, "timeline.paused", &ConfigValue::Bool(paused))
            .expect("the handler edits valid TOML")
    }

    #[test]
    fn the_key_lands_where_the_daemon_reads_it() {
        let out = set("", true);
        assert!(out.contains("[timeline]"), "{out}");
        assert!(out.contains("paused = true"), "{out}");
    }

    #[test]
    fn resuming_writes_false_rather_than_dropping_the_key() {
        // A removed key reads as the default, which is "recording", so the effect
        // would be right by accident. Written explicitly, the file says what the
        // user chose instead of leaving it to a default that could change.
        let out = set("[timeline]\npaused = true\n", false);
        assert!(out.contains("paused = false"), "{out}");
    }

    /// The read half, over the same handler the write half uses: a round trip
    /// through the file is what the switch actually depends on.
    fn reads_paused(text: &str) -> bool {
        let model = handler_for(Format::Toml).read(text).expect("valid TOML");
        matches!(model.get("timeline.paused"), Some(ConfigValue::Bool(true)))
    }

    #[test]
    fn what_was_written_is_what_is_read_back() {
        assert!(reads_paused(&set("", true)));
        assert!(!reads_paused(&set("[timeline]\npaused = true\n", false)));
    }

    #[test]
    fn a_config_without_the_section_reads_as_recording() {
        // The direction matters: an absent key must not read as paused, or the
        // switch would open claiming a pause nobody set.
        assert!(!reads_paused("[projects]\nmax_depth = 2\n"));
    }

    #[test]
    fn a_hand_written_config_keeps_its_comments_and_neighbours() {
        let before = "# where projects are looked for\n[projects]\nmax_depth = 2\n";
        let out = set(before, true);
        assert!(out.contains("# where projects are looked for"), "{out}");
        assert!(out.contains("max_depth = 2"), "{out}");
    }
}
