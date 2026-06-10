//! The per-launch cgroup v2 the confined app runs in.
//!
//! Every launch gets its own leaf cgroup named `app-arlen-<app_id>-<pid>.scope`
//! under the systemd user slice, deliberately parallel to flatpak's
//! `app-flatpak-<id>-<n>.scope` (xdg-portal `parse_cgroup` / `match_flatpak`),
//! so a future `match_arlen` in that parser is a three-line addition mirroring
//! `match_flatpak`. The cgroup gives two things:
//!
//! - **Reaping**: tearing the whole subtree down is a single write to
//!   `cgroup.kill` (kernel >= 5.14), which cannot miss a process that left the
//!   process group (the limitation `forage/build` calls out for `kill(-pid)`).
//! - **Attribution**: strand 4 records `bpf_get_current_cgroup_id()` on a
//!   file-open, and this leaf is the join key from a file-open back to the
//!   `<app_id>, <launch instance>` that produced it.
//!
//! It is a *belt* to bwrap's *braces*: bwrap's `--unshare-pid` already tears the
//! pid-namespace down when bwrap is killed, and `--die-with-parent` covers the
//! launcher dying. The cgroup adds the clean kill-all primitive and the strand-4
//! join key.
//!
//! The child joins by writing its own pid to `cgroup.procs` in `pre_exec`,
//! before Landlock (a read-only `/` ruleset would otherwise deny the write) and
//! before any seccomp filter removes the write. The parent creates the leaf and
//! reaps it.

#![cfg(target_os = "linux")]

use std::io;
use std::path::{Path, PathBuf};

/// The default cgroup v2 mount point.
pub const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// The leaf scope name for a launch: `app-arlen-<app_id>-<launch_pid>.scope`.
/// `app_id` is the validated reverse-DNS id; `launch_pid` is the launcher's pid,
/// unique per launch (reused only after the leaf is removed). Parallel to
/// flatpak's scope so xdg-portal can recover both the app identity and the
/// launch instance from the cgroup string.
pub fn cgroup_leaf_name(app_id: &str, launch_pid: u32) -> String {
    format!("app-arlen-{app_id}-{launch_pid}.scope")
}

/// Recover `(app_id, launch_pid)` from an `app-arlen-…` scope leaf, the inverse
/// of [`cgroup_leaf_name`]. Mirrors xdg-portal's `match_flatpak`: strip the
/// `app-arlen-` prefix and the `.scope` suffix, then split off the trailing
/// `-<pid>` with `rsplit_once('-')` (the pid is all digits, so a dashed app_id
/// still round-trips). The launcher only ever mints the name; this local copy is
/// the round-trip proof for the naming. NB the xdg-portal's PRODUCTION caller
/// identity comes from the frontend-supplied `method_app_id`, not cgroup
/// detection (its `parse_cgroup` is a vestigial helper, no production caller, the
/// xdg-dbus-proxy-PID problem), so a symmetric `match_arlen` there is NOT needed
/// unless the portal adopts cgroup-based identity; this is the reference IF it
/// ever does, not a pending addition.
#[allow(dead_code)]
pub fn parse_arlen_leaf(scope: &str) -> Option<(String, u32)> {
    let inner = scope.strip_suffix(".scope")?.strip_prefix("app-arlen-")?;
    let (app_id, pid) = inner.rsplit_once('-')?;
    if app_id.is_empty() {
        return None;
    }
    let pid: u32 = pid.parse().ok()?;
    Some((app_id.to_string(), pid))
}

/// The absolute path of the launch's leaf cgroup under `base` (normally
/// [`CGROUP_ROOT`]), inside the user's delegated `app.slice`.
pub fn cgroup_leaf_path(base: &Path, uid: u32, app_id: &str, launch_pid: u32) -> PathBuf {
    base.join("user.slice")
        .join(format!("user-{uid}.slice"))
        .join(format!("user@{uid}.service"))
        .join("app.slice")
        .join(cgroup_leaf_name(app_id, launch_pid))
}

/// A created leaf cgroup. Dropping it removes the (emptied) leaf directory
/// best-effort, so an early return still cleans up; a leaf left non-empty by a
/// hard launcher kill is swept by the housekeeping pass, not here.
#[derive(Debug)]
pub struct Cgroup {
    leaf: PathBuf,
}

impl Cgroup {
    /// Create the leaf cgroup directory under the real [`CGROUP_ROOT`].
    pub fn create(uid: u32, app_id: &str, launch_pid: u32) -> io::Result<Self> {
        Self::create_under(Path::new(CGROUP_ROOT), uid, app_id, launch_pid)
    }

    /// Create the leaf under an explicit base (the test seam: a tempdir stands
    /// in for the real cgroupfs).
    pub fn create_under(base: &Path, uid: u32, app_id: &str, launch_pid: u32) -> io::Result<Self> {
        let leaf = cgroup_leaf_path(base, uid, app_id, launch_pid);
        std::fs::create_dir_all(&leaf)?;
        Ok(Self { leaf })
    }

    /// The `cgroup.procs` path the child writes its pid into to join.
    pub fn procs_path(&self) -> PathBuf {
        self.leaf.join("cgroup.procs")
    }

    /// Kill every process in the subtree: write `1` to `cgroup.kill` (the clean
    /// kill-all on kernel >= 5.14), or fall back to `SIGKILL` on each pid in
    /// `cgroup.procs`. Best-effort; the leaf is removed on drop.
    pub fn kill_all(&self) {
        if std::fs::write(self.leaf.join("cgroup.kill"), b"1").is_ok() {
            return;
        }
        if let Ok(procs) = std::fs::read_to_string(self.procs_path()) {
            for line in procs.lines() {
                if let Ok(pid) = line.trim().parse::<i32>() {
                    // SAFETY: kill on an integer pid cannot corrupt memory; an
                    // already-dead pid just returns ESRCH.
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            }
        }
    }
}

impl Drop for Cgroup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.leaf);
    }
}

/// Join the current process to the cgroup whose `cgroup.procs` is `procs_path`,
/// by writing this process's pid. Called in the child's `pre_exec` before
/// Landlock. A failure is propagated so the launch fails closed rather than
/// running outside its cgroup.
pub fn join_current(procs_path: &Path) -> io::Result<()> {
    // SAFETY: getpid only reads the pid; it cannot fail or corrupt memory.
    let pid = unsafe { libc::getpid() };
    write_pid(procs_path, pid)
}

/// Write `pid` into a `cgroup.procs`-shaped file (the test seam for
/// [`join_current`]).
pub fn write_pid(procs_path: &Path, pid: i32) -> io::Result<()> {
    std::fs::write(procs_path, format!("{pid}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_name_round_trips() {
        let name = cgroup_leaf_name("com.example.app", 4321);
        assert_eq!(name, "app-arlen-com.example.app-4321.scope");
        assert_eq!(parse_arlen_leaf(&name), Some(("com.example.app".to_string(), 4321)));
    }

    #[test]
    fn leaf_name_round_trips_with_a_dashed_app_id() {
        // A reverse-DNS id may contain a dash; the numeric pid suffix has none,
        // so the rsplit-on-the-last-dash recovery is unambiguous.
        let name = cgroup_leaf_name("org.kde.app-2", 17);
        assert_eq!(parse_arlen_leaf(&name), Some(("org.kde.app-2".to_string(), 17)));
    }

    #[test]
    fn parse_rejects_foreign_scopes() {
        assert_eq!(parse_arlen_leaf("app-flatpak-org.gnome.Calculator-1.scope"), None);
        assert_eq!(parse_arlen_leaf("app-arlen-.scope"), None); // empty inner
        assert_eq!(parse_arlen_leaf("app-arlen-x-notanumber.scope"), None);
        assert_eq!(parse_arlen_leaf("app-arlen-com.a.b-5"), None); // no .scope
    }

    #[test]
    fn leaf_path_is_under_the_user_app_slice() {
        let p = cgroup_leaf_path(Path::new("/sys/fs/cgroup"), 1000, "com.a.b", 99);
        assert_eq!(
            p,
            PathBuf::from(
                "/sys/fs/cgroup/user.slice/user-1000.slice/user@1000.service/app.slice/app-arlen-com.a.b-99.scope"
            )
        );
    }

    #[test]
    fn create_join_and_kill_over_a_fake_cgroupfs() {
        // A tempdir stands in for the cgroupfs: the directory op and the
        // procs/kill writes are exercised without a real delegated cgroup.
        let base = tempfile::tempdir().unwrap();
        let cg = Cgroup::create_under(base.path(), 1000, "com.a.b", 42).unwrap();
        let leaf = cgroup_leaf_path(base.path(), 1000, "com.a.b", 42);
        assert!(leaf.is_dir());

        write_pid(&cg.procs_path(), 12345).unwrap();
        assert_eq!(std::fs::read_to_string(cg.procs_path()).unwrap().trim(), "12345");

        cg.kill_all();
        assert_eq!(std::fs::read_to_string(leaf.join("cgroup.kill")).unwrap(), "1");
    }

    #[test]
    fn drop_removes_an_empty_leaf() {
        // On a real cgroupfs the control files are kernel-virtual and do not
        // block rmdir of an emptied cgroup; here the leaf is genuinely empty.
        let base = tempfile::tempdir().unwrap();
        let leaf = cgroup_leaf_path(base.path(), 1000, "com.a.b", 7);
        {
            let _cg = Cgroup::create_under(base.path(), 1000, "com.a.b", 7).unwrap();
            assert!(leaf.is_dir());
        }
        assert!(!leaf.exists(), "drop removes the empty leaf");
    }
}
