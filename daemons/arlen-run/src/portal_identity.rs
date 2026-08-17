// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Telling the portal which app is calling.
//!
//! `xdg-desktop-portal` decides a caller's app id by reading `.flatpak-info`
//! from the caller's root, and `xdg-document-portal` files an exported document
//! under that id. Without one, an arlen-confined app is Unconfined: the picker
//! hands back a raw host path, which is unopenable once its filesystem grant is
//! narrowed to what the portal mints. That is the second half of the file
//! ruling, and this module is the half that can be written and tested.
//!
//! **The shape here was measured against the running portal, not read.** Driving
//! `Documents.Add` from inside `bwrap` with a real file descriptor and watching
//! which `by-app/<id>` the export landed under answers the question directly,
//! because that directory IS the id the portal derived. Two refusals on the way
//! pinned the format:
//!
//! - `[Application] name=` and `[Instance] instance-id=1` alone:
//!   `Failed to open file "/run/user/1000/.flatpak/1/bwrapinfo.json"`.
//! - No `[Instance]` group at all: `Key file does not have group "Instance"`.
//!
//! So the group is mandatory and it is a POINTER. The portal follows the instance
//! id to a directory holding `bwrapinfo.json`, which is bwrap's own report of the
//! sandbox it built (`--info-fd`; on 0.11.2 that is `child-pid`, `mnt-namespace`
//! and `pid-namespace`). With the instance directory bound in and that file
//! written, a confined caller's export landed at `by-app/dev.arlen.probe/...`,
//! and reading it back through the per-app bind [`crate::spawn::plumbing_binds`]
//! returned `hello` where the same read without the bind returned `No such file
//! or directory`.
//!
//! **Nothing calls this yet, on purpose.** Presenting a `.flatpak-info` tells
//! everything that looks - not only the portal - that the app is a Flatpak. GTK
//! and Qt read it to switch to the portal file chooser, which is the behaviour
//! the ruling wants; other software reads it to call `flatpak-spawn --host` or to
//! look under `/app`, and across the foreign profiles that is a change nothing
//! has measured. Whether every confined app should say this, or only the apps
//! whose file grant we narrow, is a decision rather than a commit, so the
//! mechanism lands first and the trigger lands with the answer.

use std::path::{Path, PathBuf};

/// The `.flatpak-info` a confined app presents, naming itself and pointing at its
/// instance directory.
///
/// Both groups are required: the portal refuses a file without `[Instance]`, and
/// resolves `instance-id` to the directory [`instance_dir`] names. Only the two
/// keys the portal was measured to need are written - a fuller file would be
/// claiming runtime and branch details that are not true of an Arlen app.
pub fn flatpak_info(app_id: &str, instance_id: &str) -> String {
    format!("[Application]\nname={app_id}\n\n[Instance]\ninstance-id={instance_id}\n")
}

/// Where the portal looks for `bwrapinfo.json` after following `instance-id`.
///
/// Under `$XDG_RUNTIME_DIR/.flatpak/` because that is the path the portal
/// resolves; the name is not ours to choose.
pub fn instance_dir(runtime_dir: &Path, instance_id: &str) -> PathBuf {
    runtime_dir.join(".flatpak").join(instance_id)
}

/// The file `bwrap --info-fd` writes and the portal reads.
pub fn bwrapinfo_path(runtime_dir: &Path, instance_id: &str) -> PathBuf {
    instance_dir(runtime_dir, instance_id).join("bwrapinfo.json")
}

/// A launch's instance id: the app id and the launcher's pid.
///
/// Unique per launch (one `arlen-run` per app launch) and self-describing in a
/// directory listing, which matters because a crashed launcher leaves its
/// instance directory behind and someone has to be able to tell what it was.
/// Dots and dashes are all `app_id` can hold and all this adds, so the result is
/// a safe path component without further escaping.
pub fn instance_id(app_id: &str, launcher_pid: u32) -> String {
    format!("{app_id}-{launcher_pid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pinned against the file the portal actually accepted. A change here is a
    /// change to what the portal is told, so it should be measured again rather
    /// than adjusted to match a new expectation.
    #[test]
    fn the_info_file_is_the_one_the_portal_accepted() {
        assert_eq!(
            flatpak_info("dev.arlen.files", "dev.arlen.files-4211"),
            "[Application]\nname=dev.arlen.files\n\n[Instance]\ninstance-id=dev.arlen.files-4211\n"
        );
    }

    /// Omitting the group was measured to fail with `Key file does not have group
    /// "Instance"`, so its presence is not decoration.
    #[test]
    fn the_instance_group_is_present() {
        let info = flatpak_info("dev.arlen.files", "x");
        assert!(
            info.contains("[Instance]"),
            "the portal refuses a file without it"
        );
        assert!(info.contains("instance-id=x"));
    }

    /// The id the app claims is the id the portal files its documents under, and
    /// `plumbing_binds` binds `by-app/<app_id>` - so a mismatch would bind one
    /// directory and export into another.
    #[test]
    fn the_claimed_name_is_the_app_id_the_bind_uses() {
        let info = flatpak_info("dev.arlen.files", "dev.arlen.files-1");
        assert!(info.contains("\nname=dev.arlen.files\n"));
    }

    #[test]
    fn the_instance_directory_is_where_the_portal_looks() {
        let rt = Path::new("/run/user/1000");
        assert_eq!(
            bwrapinfo_path(rt, "dev.arlen.files-4211"),
            PathBuf::from("/run/user/1000/.flatpak/dev.arlen.files-4211/bwrapinfo.json")
        );
    }

    /// Two launches of one app must not share an instance directory: the second
    /// would overwrite the first's `bwrapinfo.json` while the first is still
    /// running, and the portal would then resolve it to the wrong namespace.
    #[test]
    fn two_launches_of_one_app_get_separate_instances() {
        assert_ne!(
            instance_id("dev.arlen.files", 4211),
            instance_id("dev.arlen.files", 4212)
        );
    }

    /// It has to be a single path component; a separator would put the directory
    /// somewhere other than where the portal looks.
    #[test]
    fn the_instance_id_is_one_path_component() {
        let id = instance_id("dev.arlen.files", 4211);
        assert!(
            !id.contains('/'),
            "{id} would escape the instance directory"
        );
        assert_eq!(
            instance_dir(Path::new("/run/user/1000"), &id)
                .components()
                .count(),
            Path::new("/run/user/1000/.flatpak").components().count() + 1
        );
    }
}
