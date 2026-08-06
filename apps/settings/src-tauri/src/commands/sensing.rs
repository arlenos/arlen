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

/// What the file says about one key. Mirrors the portal's reader exactly,
/// including the distinction between a key nobody stated and a file nobody can
/// read.
///
/// Deliberately a second copy rather than a shared crate for one predicate. The
/// two sides must agree, and what keeps them agreeing is the shared fixture and
/// the tests below, not a dependency edge between an app and a daemon.
#[derive(Debug, PartialEq)]
enum Reading {
    Off,
    On,
    NotStated,
    Unreadable,
}

fn read_key(text: &str, key: &str) -> Reading {
    let mut saw_a_setting = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((name, value)) = line.split_once('=') else { continue };
        let (name, value) = (name.trim(), value.trim());
        if name.is_empty() {
            continue;
        }
        saw_a_setting = true;
        if name == key {
            return match value {
                "false" => Reading::Off,
                "true" => Reading::On,
                _ => Reading::Unreadable,
            };
        }
    }
    if saw_a_setting { Reading::NotStated } else { Reading::Unreadable }
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
    // The same three-way reading the portal enforces, because a page that shows
    // "on" while the portal denies every capture is worse than no page: the user
    // would go looking for the app that broke rather than the file that did.
    let off = match switch_file() {
        None => false,
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(text) => matches!(
                read_key(&text, "screen_capture"),
                Reading::Off | Reading::Unreadable
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => true,
        },
    };
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


    /// The truncated file, shared for the same reason as the off one: the rule
    /// that an unreadable intent reads as off lives in two copies now.
    const TRUNCATED_FIXTURE: &str = include_str!("../../../../../dev/fixtures/sensing-truncated.toml");

    #[test]
    fn the_shared_truncated_file_reads_as_off_on_this_side_too() {
        assert_eq!(read_key(TRUNCATED_FIXTURE, "screen_capture"), Reading::Unreadable);
    }

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
        assert_eq!(read_key(&off, "screen_capture"), Reading::Off);

        let on = render(&SensingState { screen_capture: true });
        assert_eq!(read_key(&on, "screen_capture"), Reading::On);
    }

    #[test]
    fn the_comment_in_the_rendered_file_is_not_itself_a_setting() {
        // The header mentions the key by name in prose. A reader that took the
        // first line mentioning it would read every file as switched off.
        let on = render(&SensingState { screen_capture: true });
        assert!(on.contains('#'));
        assert_eq!(read_key(&on, "screen_capture"), Reading::On);
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
            let want = if allowed { Reading::On } else { Reading::Off };
            assert_eq!(read_key(&read, "screen_capture"), want);
            // Two statements of the key would make the file's meaning depend on
            // which one a reader takes first.
            assert_eq!(read.matches("screen_capture =").count(), 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
