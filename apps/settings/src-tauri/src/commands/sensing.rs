// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: GPL-3.0-only

//! Reading and setting the sensing master switches.
//!
//! The switch is enforced in the portal, where capture is exercised
//! (living-capability-graph.md 2a). This is only the surface that reads and
//! writes the user's choice; nothing here decides whether a capture happens, and
//! nothing that skips this file can be trusted to have been switched off.
//!
//! **The write is atomic, and that is load-bearing rather than tidy.** The portal
//! re-reads the file on every check, so a plain truncate-and-write leaves a window
//! where it reads a half-written file. Its parser treats anything it cannot read
//! as "on" - correctly, because a system with no switch file is a working system -
//! so a torn read would silently turn capture back on for as long as the write
//! took. Writing a sibling temp file and renaming means a reader sees the old
//! contents or the new one, never neither.

use std::io::Write;
use std::path::PathBuf;

/// Which switches exist, and where they stand.
///
/// One field per member of the sensing class. Camera and microphone are absent
/// rather than reported as `true`: there is no portal for either, so a switch
/// would report a protection nothing enforces, which is the failure this feature
/// exists to avoid. They arrive with their portals.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensingState {
    /// Whether screen capture is allowed system-wide.
    pub screen_capture: bool,
}

fn switch_file() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|c| c.join("arlen/sensing.toml"))
}

/// Whether the file switches a key off. Mirrors the portal's reader exactly:
/// only an explicit `false` is off, everything else is on.
///
/// Deliberately a second copy of four lines rather than a shared crate for one
/// predicate. The two sides must agree, and the thing that keeps them agreeing is
/// the test below asserting this side's answers, not a dependency edge between an
/// app and a daemon.
fn key_is_false(text: &str, key: &str) -> bool {
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((name, value)) = line.split_once('=') else { continue };
        if name.trim() == key {
            return value.trim() == "false";
        }
    }
    false
}

/// Render the whole file from state.
///
/// The file is rewritten rather than edited, because it holds this one class and
/// nothing else; there are no neighbouring settings or comments to preserve, and
/// a format-preserving edit would be machinery guarding nothing.
fn render(state: &SensingState) -> String {
    format!(
        "# Sensing master switches. Off subtracts from every app at once and is\n\
         # enforced where the capability is exercised, not in the settings UI.\n\
         screen_capture = {}\n",
        state.screen_capture
    )
}

/// Where the switches stand.
#[tauri::command]
pub async fn settings_sensing_state() -> SensingState {
    let off = switch_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| key_is_false(&t, "screen_capture"))
        .unwrap_or(false);
    SensingState {
        screen_capture: !off,
    }
}

/// Set the screen-capture switch.
///
/// The interaction weight is the caller's: turning a protection off is one click,
/// turning it back on is at least as heavy as granting the capability, because it
/// restores sensing to everyone holding a grant. That asymmetry lives in the page
/// - a backend that refused to enable without a token would be inventing a
/// consent mechanism next to the one the system already has.
#[tauri::command]
pub async fn settings_sensing_set_screen_capture(allowed: bool) -> Result<(), String> {
    let path = switch_file().ok_or("no config directory to write to")?;
    let parent = path.parent().ok_or("switch file has no parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;

    let body = render(&SensingState {
        screen_capture: allowed,
    });

    // A sibling temp file, so the rename is within one filesystem and therefore
    // atomic. `.tmp` next to the target rather than in /tmp for the same reason.
    let tmp = path.with_extension("toml.tmp");
    {
        let mut file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        file.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
        // Flushed before the rename: a rename that publishes a file whose bytes
        // are still in the page cache would survive a crash as an empty file,
        // which this reader would take as "on".
        file.sync_all().map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared fixture, held at `dev/fixtures/` so the portal's tests read the
    /// same bytes. See that directory's README for why the predicate is copied
    /// rather than shared.
    const OFF_FIXTURE: &str = include_str!("../../../../../dev/fixtures/sensing-off.toml");

    #[test]
    fn the_off_file_is_rendered_exactly_as_the_portal_expects_it() {
        // The other tests check this side against its own parser, which would
        // still pass if both copies drifted together. This one is anchored to
        // bytes the portal also asserts against.
        assert_eq!(render(&SensingState { screen_capture: false }), OFF_FIXTURE);
    }

    #[test]
    fn what_is_rendered_is_what_the_portal_reads_back() {
        // The two sides have separate copies of the predicate, so the round trip
        // is the thing that keeps them honest. If either drifts, this fails.
        let off = render(&SensingState { screen_capture: false });
        assert!(key_is_false(&off, "screen_capture"));

        let on = render(&SensingState { screen_capture: true });
        assert!(!key_is_false(&on, "screen_capture"));
    }

    #[test]
    fn the_comment_in_the_rendered_file_is_not_itself_a_setting() {
        // The header mentions the key by name in prose. A reader that took the
        // first line mentioning it would read every file as switched off.
        let on = render(&SensingState { screen_capture: true });
        assert!(on.contains('#'));
        assert!(!key_is_false(&on, "screen_capture"));
    }

    #[test]
    fn a_write_replaces_rather_than_appends() {
        let dir = std::env::temp_dir().join(format!("arlen-sensing-w-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sensing.toml");

        for allowed in [false, true, false] {
            let body = render(&SensingState { screen_capture: allowed });
            let tmp = path.with_extension("toml.tmp");
            std::fs::write(&tmp, &body).unwrap();
            std::fs::rename(&tmp, &path).unwrap();
            let read = std::fs::read_to_string(&path).unwrap();
            assert_eq!(key_is_false(&read, "screen_capture"), !allowed);
            // Two statements of the key would make the file's meaning depend on
            // which one a reader takes first.
            assert_eq!(read.matches("screen_capture =").count(), 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
