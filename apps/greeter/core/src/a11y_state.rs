// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! What the login screen remembers about itself.
//!
//! Not a user's preference, and the distinction is the whole design. This
//! records *this login screen was last used with a screen reader* - a fact about
//! the machine's front door, kept by the front door. It is never read from
//! anyone's user config and never written into one.
//!
//! It has to exist because of a wall that is not going away: the config broker
//! authenticates exactly one uid, the session user's, and the greeter is a
//! different user. Before login no user has been chosen, so a greeter that could
//! read one person's config could read anyone's. Refusing that is the auth
//! working. But somebody who cannot see the screen has to be able to reach the
//! login prompt at all, and being made to find the same toggle again at every
//! boot is precisely the exclusion this is meant to remove.
//!
//! WHAT IT LEAKS, said plainly: on a shared machine the login screen shows the
//! last person's accessibility choice to everyone who walks up. That is the
//! unavoidable cost of a front door a returning blind person can use, and it is
//! bounded to one boolean about the door rather than anything about a user.
//!
//! It is deliberately NOT the same bit as the session's. The session's lives in
//! that user's own broker; this one lives here. They meet only in one direction,
//! when somebody operates the toggle at a login and the choice travels forward
//! with the session they start.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The file, under the state directory.
const STATE_FILE: &str = "a11y.toml";

/// Override for tests and a dev run, where `/var/lib` is neither present nor
/// writable. Debug-only: a release greeter always uses the real path, so a
/// same-machine process cannot redirect it by setting a variable.
#[cfg(debug_assertions)]
pub const STATE_DIR_ENV: &str = "ARLEN_GREETER_STATE_DIR";

/// The accessibility options this login screen remembers.
///
/// One field, matching what the session carries. The rest of the greeter's
/// toggles (contrast, large text, on-screen keyboard) are deliberately absent:
/// they are re-reachable without sight or a reader, so nothing is lost by their
/// not persisting, and a field nothing has asked for is a promise rather than a
/// feature.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GreeterA11y {
    /// True when this login screen was last operated with a screen reader on.
    pub screen_reader: bool,
}

/// Where the state lives.
///
/// `/var/lib/arlen/greeter` follows the tree's convention for daemon-owned
/// state. The greeter is not yet deployed with a unit of its own (the image
/// autologins today), so this is the convention rather than a path some
/// `StateDirectory=` already creates - which is why the loader treats an absent
/// directory as "nothing remembered" rather than an error.
///
/// WHOEVER WRITES THAT DEPLOYMENT HAS TO PROVISION THIS, and it will not work by
/// accident: `/var/lib/arlen` is `0755 root root` (see
/// `daemons/config-broker/dist/arlen-config-broker.tmpfiles.conf`), so the
/// greeter's user cannot create a subdirectory under it however hard
/// `create_dir_all` tries. Without a tmpfiles entry owned by whatever user the
/// greeter runs as, every write here fails with EACCES and the login screen
/// tells the person - correctly and forever - that it could not save their
/// choice.
///
/// `apps/greeter/dist/arlen-greeter.tmpfiles.conf` is that entry. It names
/// `_greetd`, which is not the guess this comment used to refuse to make: it is
/// the system user greetd's own Debian package creates, and the user its shipped
/// `/etc/greetd/config.toml` runs `[default_session]` as. Read off the package,
/// not off a convention.
///
/// It is not installed anywhere yet, because nothing installs the greeter. This
/// image's greetd runs `arlen-session` as `arlen` for both sessions and starts
/// no greeter at all, so provisioning a directory here today would be setting a
/// table for a program that is not coming. The file ships with the greeter so it
/// arrives with it. Note it cannot be a unit's `StateDirectory=`, the way every
/// other component in this tree gets its state directory: greetd spawns the
/// greeter as a command, so there is no service for systemd to create one for.
pub fn state_dir() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var(STATE_DIR_ENV) {
        return PathBuf::from(dir);
    }
    PathBuf::from("/var/lib/arlen/greeter")
}

/// What this login screen remembers, or the default when it remembers nothing.
///
/// ABSENT AND UNREADABLE BOTH RESOLVE TO THE DEFAULT, which is the one place
/// this file departs from the tree's usual absent/refused split, on purpose: a
/// login screen that refuses to draw because it could not read one boolean is
/// worse for everybody than one that draws with the toggle off, and the toggle
/// is right there. The unreadable case is logged by the caller rather than
/// swallowed.
pub fn load_in(dir: &Path) -> Result<GreeterA11y, String> {
    let path = dir.join(STATE_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(GreeterA11y::default()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Remember this, for the next time the machine boots.
///
/// Written the moment the toggle is operated, not on a successful login:
/// somebody who switches the reader on and then mistypes their password still
/// needs it there when they try again.
pub fn store_in(dir: &Path, state: GreeterA11y) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(STATE_FILE);
    let text = toml::to_string_pretty(&state).map_err(|e| e.to_string())?;
    // Sibling temp + rename: a login screen that dies mid-write must leave the
    // previous answer intact rather than a half-file that fails to parse.
    // Per INSTANCE, not per app: nothing stops a second window, and two of them
    // sharing one temp name do not tear the file - they cross over, and one
    // renames the other's bytes into place (`app-instance-model.md`).
    let tmp = dir.join(format!(".{STATE_FILE}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_remembered_is_the_default_not_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(load_in(tmp.path()).unwrap(), GreeterA11y::default());
        assert!(!load_in(tmp.path()).unwrap().screen_reader);
    }

    #[test]
    fn what_was_switched_on_survives_a_reboot() {
        // The whole point: reaching the login screen unaided, once.
        let tmp = tempfile::tempdir().unwrap();
        store_in(tmp.path(), GreeterA11y { screen_reader: true }).unwrap();
        assert!(load_in(tmp.path()).unwrap().screen_reader);

        // ...and switching it back off survives too, rather than sticking on.
        store_in(tmp.path(), GreeterA11y { screen_reader: false }).unwrap();
        assert!(!load_in(tmp.path()).unwrap().screen_reader);
    }

    #[test]
    fn a_corrupt_file_is_reported_rather_than_read_as_off() {
        // Silently reading it as off would be the login screen forgetting
        // somebody's only way in, with nothing said about why.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(STATE_FILE), "this = is = not = toml").unwrap();
        assert!(load_in(tmp.path()).is_err());
    }

    #[test]
    fn the_write_leaves_no_temp_behind() {
        let tmp = tempfile::tempdir().unwrap();
        store_in(tmp.path(), GreeterA11y { screen_reader: true }).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != STATE_FILE)
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }
}
