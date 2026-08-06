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
//! **An absent file and an unreadable one are opposite states.** No file means
//! nobody configured anything, so capture works. A file that exists but cannot be
//! read - truncated, corrupted, unreadable - means somebody stated an intent that
//! can no longer be read, and the safe reading of an unreadable intent is the
//! protective one. Collapsing the two would let a corrupted file silently resume
//! capture for a user whose whole belief is that they switched it off, which is
//! the one failure a master switch must not have.
//!
//! **Read on every check, never cached.** The intent is "off, right now", so a
//! value read once at startup would keep sensing alive for whatever remains of
//! the session. The read is a small file and the checks are per-request, not
//! per-frame.
//!
//! This is enforcement, not display. A switch that greys out a row while the
//! device still streams reports a protection it does not provide.

use std::path::PathBuf;

/// Where the shared vector table lives, relative to this file.
const VECTOR_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../dev/fixtures/sensing-vectors");

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

/// What the file says about one key.
///
/// Three answers rather than a boolean, because "the file does not say" and "the
/// file is not sayable" need opposite treatment and a `bool` collapses them.
#[derive(Debug, PartialEq)]
enum Reading {
    /// The key is stated off.
    Off,
    /// The key is stated on.
    On,
    /// The key is not stated, but the file does state other settings. It is a
    /// switch file for something else and this capability is unconfigured.
    NotStated,
    /// Nothing in the file parses as a setting, or the key's value is neither
    /// `true` nor `false`. Someone wrote something here and it cannot be read.
    Unreadable,
}

/// Read one boolean key from the flat switch file.
///
/// A hand parser rather than a TOML dependency for one file of booleans, and
/// strict about values: only exact `true` and `false` are readable, so a write
/// truncated to `screen_capture = fal` is an unreadable intent rather than a
/// silent "not off".
///
/// The distinction that earns the enum: a file truncated to nothing parses
/// cleanly and mentions no key, which is indistinguishable from a file about a
/// different switch unless the parser also reports whether it read ANY setting.
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

/// Whether screen capture is switched off system-wide.
///
/// Note the direction: this answers "is it off", and everything that is not a
/// clear off is on. An enforcement check that failed closed would take the
/// desktop's screen sharing away on an unreadable file, which is a bigger harm
/// than the switch prevents - and unlike a grant, the absence of a switch is not
/// the absence of permission.
pub fn screen_capture_is_off() -> bool {
    let Some(path) = switch_file() else { return false };
    match std::fs::read_to_string(&path) {
        Ok(text) => matches!(
            read_key(&text, "screen_capture"),
            Reading::Off | Reading::Unreadable
        ),
        // No file: nobody configured anything, and a system nobody configured is
        // a working system.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        // A file that exists and cannot be read is the case this branch exists
        // for. Somebody wrote an intent here; the only safe reading of an intent
        // nobody can read is the protective one. Keyed on the error kind rather
        // than a second `exists()` call, which would answer about a different
        // moment than the read did.
        Err(_) => true,
    }
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
    fn a_stated_value_is_read_as_stated() {
        assert_eq!(read_key("screen_capture = false", "screen_capture"), Reading::Off);
        assert_eq!(read_key("screen_capture=false\n", "screen_capture"), Reading::Off);
        assert_eq!(read_key("screen_capture = true", "screen_capture"), Reading::On);
    }

    #[test]
    fn a_file_about_other_switches_leaves_this_one_unconfigured() {
        // Not `Unreadable`: the file parses, it simply is not about this
        // capability. Treating it as off would switch screen capture off the day
        // a microphone switch ships.
        assert_eq!(read_key("microphone = false", "screen_capture"), Reading::NotStated);
    }

    #[test]
    fn a_file_that_parses_as_nothing_is_unreadable_rather_than_silent() {
        // The failure this whole enum exists for: a write truncated to empty, to
        // the header comment, or to half a key name. Each of these was "not off"
        // before, so a corrupted file resumed capture without telling anyone.
        assert_eq!(read_key("", "screen_capture"), Reading::Unreadable);
        assert_eq!(read_key("# Sensing master switches.\n", "screen_capture"), Reading::Unreadable);
        assert_eq!(read_key("screen_captu", "screen_capture"), Reading::Unreadable);
        assert_eq!(read_key("= false\n", "screen_capture"), Reading::Unreadable);
    }

    #[test]
    fn a_value_truncated_mid_word_is_unreadable_rather_than_not_false() {
        // `fal` is not `false`, and a parser that only asked "is it false" would
        // read this as on. It is a half-written off.
        assert_eq!(read_key("screen_capture = fal", "screen_capture"), Reading::Unreadable);
        assert_eq!(read_key("screen_capture = ", "screen_capture"), Reading::Unreadable);
        assert_eq!(read_key("screen_capture = FALSE", "screen_capture"), Reading::Unreadable);
    }

    #[test]
    fn a_comment_is_not_a_setting() {
        assert_eq!(read_key("# screen_capture = false", "screen_capture"), Reading::Unreadable);
        assert_eq!(
            read_key("screen_capture = false # turned off in the meeting", "screen_capture"),
            Reading::Off
        );
    }

    #[test]
    fn the_first_statement_of_a_key_wins() {
        // Whichever way this goes it must be decided rather than incidental; a
        // later line silently overriding an earlier one hides a duplicated key.
        assert_eq!(
            read_key("screen_capture = false\nscreen_capture = true", "screen_capture"),
            Reading::Off
        );
        assert_eq!(
            read_key("screen_capture = true\nscreen_capture = false", "screen_capture"),
            Reading::On
        );
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


    /// The truncated file, shared for the same reason as the off one: the rule
    /// that an unreadable intent reads as off lives in two copies now.
    const TRUNCATED_FIXTURE: &str = include_str!("../../../../dev/fixtures/sensing-truncated.toml");

    #[test]
    fn the_shared_truncated_file_reads_as_off_on_this_side_too() {
        assert_eq!(read_key(TRUNCATED_FIXTURE, "screen_capture"), Reading::Unreadable);
    }

    #[test]
    fn the_file_settings_writes_reads_as_off() {
        assert_eq!(read_key(OFF_FIXTURE, "screen_capture"), Reading::Off);
    }

    #[test]
    fn the_fixtures_header_does_not_do_the_switching() {
        // The header names the key in prose on both comment lines. A reader that
        // matched the first mention would call every file off, including one
        // whose value says true, so the fixture is the guard against that too.
        let on = OFF_FIXTURE.replace("screen_capture = false", "screen_capture = true");
        assert_eq!(read_key(&on, "screen_capture"), Reading::On);
    }
}

#[cfg(test)]
mod file_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

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

        // A file that exists but says nothing readable: this is the corrupted
        // write, and it must NOT read as the absent case two lines above.
        std::fs::write(dir.join("arlen/sensing.toml"), "").unwrap();
        assert!(screen_capture_is_off(), "a truncated file must not resume capture");

        // Unreadable rather than unparseable, which reaches the other branch: the
        // read fails with a kind that is not NotFound. Skipped under a uid that
        // ignores the mode, where the premise does not hold.
        std::fs::write(dir.join("arlen/sensing.toml"), "screen_capture = true\n").unwrap();
        let locked = dir.join("arlen/sensing.toml");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_to_string(&locked).is_err() {
            assert!(screen_capture_is_off(), "an unreadable file must not resume capture");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();

        match previous {
            Some(v) => unsafe { std::env::set_var("XDG_CONFIG_HOME", v) },
            None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod vector_tests {
    use super::*;

    /// Every implementation of this predicate runs against the same table of
    /// files, held at `dev/fixtures/sensing-vectors/`. The copies are not merged -
    /// a cross-repo release dependency for four lines costs more than it buys -
    /// so what keeps them from diverging is that all three answer this table.
    ///
    /// The expected reading is the part of each filename before `__`, so a case is
    /// added by dropping in a file and no code changes anywhere.
    #[test]
    fn every_reader_agrees_with_the_shared_vector_table() {
        let dir = std::path::Path::new(VECTOR_DIR);
        let entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap_or_else(|e| panic!("vector table missing at {}: {e}", dir.display()))
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "toml"))
            .collect();
        assert!(entries.len() >= 12, "the table lost cases: {} left", entries.len());

        for entry in entries {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let (want, _) = name.split_once("__").unwrap_or_else(|| {
                panic!("vector {name} is not named <expected>__<case>.toml")
            });
            let text = std::fs::read_to_string(&path).unwrap();
            let got = read_key(&text, "screen_capture");
            let expected = match want {
                "off" => Reading::Off,
                "on" => Reading::On,
                "not-stated" => Reading::NotStated,
                "unreadable" => Reading::Unreadable,
                other => panic!("vector {name} claims an answer nobody defines: {other}"),
            };
            assert_eq!(got, expected, "{name}");
        }
    }
}
