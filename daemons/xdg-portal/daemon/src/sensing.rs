// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: GPL-3.0-only

//! The sensing master switch, enforced where the capability is exercised.
//!
//! A master switch subtracts from every principal at once: off means no screen
//! capture for anyone, regardless of what any app was granted. That asymmetry is
//! what makes it compatible with the rule that authority is only ever conferred
//! through the capability graph - a switch never confers, so it creates no
//! authority the grant browser cannot see (living-capability-graph.md 2a).
//!
//! It is deliberately narrow. The test for whether a capability deserves one is
//! whether *"off, right now, regardless of who asked"* is a coherent intent that
//! per-app revocation cannot express. For a microphone it plainly is; for "read
//! files" it only breaks the machine. Screen capture is the one member that is
//! live today, because the portal implements ScreenCast and Screenshot and there
//! is no camera or microphone portal yet - those switches ship with those portals.
//!
//! **Read on every check, never cached.** The intent is "off, right now", so a
//! value read once at startup would keep sensing alive for whatever remains of
//! the session. The read is a small file and the checks are per-request, not
//! per-frame.
//!
//! This is enforcement, not display. A switch that greys out a row while the
//! device still streams reports a protection it does not provide.

use std::path::PathBuf;

/// The file holding the user's switch positions.
///
/// User intent rather than machine state - somebody flicked a switch - so it
/// lives with the other things they chose, not in a runtime directory that a
/// reboot clears.
fn switch_file() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
        })
        .map(|c| c.join("arlen/sensing.toml"))
}

/// Read one boolean key from the flat switch file.
///
/// A hand parser rather than a TOML dependency for one file of booleans, and
/// deliberately strict: only an exact `false` turns something off, so a typo, a
/// truncated write or a comment can never silently disable capture. Every other
/// outcome - absent file, absent key, unparsable line - leaves the capability on,
/// because the default state of a system with no switch file is working.
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

/// Whether screen capture is switched off system-wide.
///
/// Note the direction: this answers "is it off", and everything that is not a
/// clear off is on. An enforcement check that failed closed would take the
/// desktop's screen sharing away on an unreadable file, which is a bigger harm
/// than the switch prevents - and unlike a grant, the absence of a switch is not
/// the absence of permission.
pub fn screen_capture_is_off() -> bool {
    let Some(path) = switch_file() else { return false };
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    key_is_false(&text, "screen_capture")
}

/// What a caller is told when the switch is off.
///
/// Names the switch rather than the app's grant, because the app's grant is fine
/// and re-requesting it will not help. A message about permission would send
/// someone to the wrong screen.
pub const SCREEN_CAPTURE_OFF: &str = "screen capture is switched off for this system";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_explicit_false_switches_a_capability_off() {
        assert!(key_is_false("screen_capture = false", "screen_capture"));
        assert!(key_is_false("screen_capture=false\n", "screen_capture"));
        assert!(!key_is_false("screen_capture = true", "screen_capture"));
    }

    #[test]
    fn a_file_that_says_nothing_about_it_leaves_it_on() {
        // The failure that matters: a truncated or half-written file must not
        // read as "off" for one capability and leave the others alone by luck.
        assert!(!key_is_false("", "screen_capture"));
        assert!(!key_is_false("microphone = false", "screen_capture"));
        assert!(!key_is_false("screen_capture", "screen_capture"));
        assert!(!key_is_false("screen_capture = fals", "screen_capture"));
    }

    #[test]
    fn a_comment_is_not_a_setting() {
        assert!(!key_is_false("# screen_capture = false", "screen_capture"));
        assert!(key_is_false("screen_capture = false # turned off in the meeting", "screen_capture"));
    }

    #[test]
    fn the_first_statement_of_a_key_wins() {
        // Whichever way this goes it must be decided rather than incidental; a
        // later line silently overriding an earlier one hides a duplicated key.
        assert!(key_is_false("screen_capture = false\nscreen_capture = true", "screen_capture"));
        assert!(!key_is_false("screen_capture = true\nscreen_capture = false", "screen_capture"));
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    /// The bytes Settings writes for the off position, held once at
    /// `dev/fixtures/` so neither side can change the format without the other
    /// noticing. Settings asserts it renders exactly this; this asserts it reads
    /// as off. The predicate is copied on both sides deliberately - see that
    /// directory's README - and this is what keeps the copies honest.
    const OFF_FIXTURE: &str = include_str!("../../../../dev/fixtures/sensing-off.toml");

    #[test]
    fn the_file_settings_writes_reads_as_off() {
        assert!(key_is_false(OFF_FIXTURE, "screen_capture"));
    }

    #[test]
    fn the_fixtures_header_does_not_do_the_switching() {
        // The header names the key in prose on both comment lines. A reader that
        // matched the first mention would call every file off, including one
        // whose value says true, so the fixture is the guard against that too.
        let on = OFF_FIXTURE.replace("screen_capture = false", "screen_capture = true");
        assert!(!key_is_false(&on, "screen_capture"));
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;

    /// The parser tests prove the rule; this proves the wiring, which is the half
    /// that would otherwise be assumed. A switch whose file is never actually read
    /// is the false-green this whole feature exists to avoid.
    ///
    /// Serial by construction: it sets `XDG_CONFIG_HOME`, which the whole test
    /// binary shares, so it is one test that sets, reads and restores rather than
    /// several that race.
    #[test]
    fn the_switch_file_is_read_from_the_config_directory() {
        let dir = std::env::temp_dir().join(format!("arlen-sensing-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("arlen")).unwrap();
        let previous = std::env::var_os("XDG_CONFIG_HOME");

        // SAFETY: single-threaded within this test, and the value is restored
        // before it returns. Rust 2024 marks these unsafe for exactly the
        // cross-thread reason, which is why this is the only test that sets one.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };

        // No file at all: capture works, because a system nobody has configured
        // is a working system.
        assert!(!screen_capture_is_off());

        std::fs::write(dir.join("arlen/sensing.toml"), "screen_capture = false\n").unwrap();
        assert!(screen_capture_is_off(), "the file must reach the enforcement point");

        // And back on again, read fresh rather than from a cached first answer -
        // "off, right now" has to mean "on, right now" too.
        std::fs::write(dir.join("arlen/sensing.toml"), "screen_capture = true\n").unwrap();
        assert!(!screen_capture_is_off(), "the value is re-read, not cached");

        match previous {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
