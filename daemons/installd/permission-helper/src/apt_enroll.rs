//! The `.deb` apt-enroll orchestrator (app-enrollment §E4).
//!
//! apt streams a `DPkg::Pre-Install-Pkgs` block to a configured hook on stdin.
//! installd's `apt_hook` module parses that stream and pairs each installed
//! package with a curated starting profile; this is the half that ACTS on the
//! result, writing each matched profile into the root-owned system tier so the
//! package is confined from its first launch rather than after someone notices.
//!
//! It lives here rather than in installd because the hook runs as **root**, in
//! apt's process, with no session bus to reach: installd is a per-user session
//! service and cannot write `/var/lib/arlen/permissions`. The validated writer it
//! needs ([`crate::profile::write_profile_in`]) is already in this crate.
//!
//! # Which uid gets the profile
//!
//! A `.deb` is machine-wide but a profile is per-uid, so one enrolment fans out
//! to every human uid on the machine. `enroll_stream` takes them explicitly
//! rather than reading `/etc/passwd` itself, so the policy of who counts as a
//! human user stays with the caller and the orchestration is testable.
//!
//! # Failure posture
//!
//! An apt hook that exits non-zero can abort the package operation, so a
//! per-package failure is REPORTED and skipped, never fatal: failing to confine
//! one package must not leave the machine unable to install software. The
//! outcome is returned so the binary can log exactly what was and was not
//! enrolled. That is the deliberate trade - an unenrolled package is visible in
//! the log and still covered by the learning-mode fallback (§E9), whereas a
//! blocked `apt install` is an outage.

use std::path::{Path, PathBuf};

/// What happened to one package the stream asked to enroll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enrolled {
    /// The curated profile was written for these uids.
    Written {
        /// The `.deb` package name, which is also the profile's app id.
        package: String,
        /// Where it landed, one path per uid.
        paths: Vec<PathBuf>,
    },
    /// A curated profile matched but could not be written.
    Failed {
        /// The `.deb` package name.
        package: String,
        /// Why, for the hook's log.
        reason: String,
    },
}

/// Read a curated profile and write it into the system tier for each uid.
///
/// The content is copied verbatim: a curated profile is the reviewed starting
/// grant, and rewriting it here would mean two sources of truth for what a
/// package may do. `write_profile_in` re-validates the app id and the TOML, so a
/// malformed curated file is refused rather than persisted.
fn enroll_one(
    package: &str,
    curated: &Path,
    uids: &[u32],
    base: &Path,
) -> Result<Vec<PathBuf>, String> {
    let content = std::fs::read_to_string(curated)
        .map_err(|e| format!("cannot read curated profile {}: {e}", curated.display()))?;
    let mut paths = Vec::with_capacity(uids.len());
    for uid in uids {
        let path = crate::profile::write_profile_in(base, *uid, package, &content)
            .map_err(|e| format!("cannot write profile for uid {uid}: {e}"))?;
        paths.push(path);
    }
    Ok(paths)
}

/// Enroll every package in an apt `Pre-Install-Pkgs` stream that has a curated
/// starting profile.
///
/// `matched` is the output of installd's `apt_hook::match_enrollments` (package
/// name paired with its curated profile path), passed in rather than re-parsed
/// so this crate does not depend on installd's parser and the two halves stay
/// independently testable.
///
/// Returns one [`Enrolled`] per matched package, in stream order. A package with
/// no curated profile never reaches here; it is left to learning mode.
pub fn enroll_matched(
    matched: &[(String, PathBuf)],
    uids: &[u32],
    base: &Path,
) -> Vec<Enrolled> {
    matched
        .iter()
        .map(|(package, curated)| match enroll_one(package, curated, uids, base) {
            Ok(paths) => Enrolled::Written {
                package: package.clone(),
                paths,
            },
            Err(reason) => Enrolled::Failed {
                package: package.clone(),
                reason,
            },
        })
        .collect()
}

/// The human uids a machine-wide package enrolment fans out to: every account in
/// `/etc/passwd` inside the conventional login-uid range with a real shell.
///
/// System accounts are excluded because they do not launch desktop applications,
/// and writing them a profile would leave dead files that later read as an
/// enrolled app. `nobody` (65534) is above the range and excluded with them.
pub fn human_uids(passwd: &str) -> Vec<u32> {
    const MIN_UID: u32 = 1000;
    const MAX_UID: u32 = 60000;
    let mut uids = Vec::new();
    for line in passwd.lines() {
        let mut fields = line.split(':');
        let (_name, _pw, uid, _gid, _gecos, _home, shell) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next().unwrap_or(""),
        );
        let Some(uid) = uid.and_then(|u| u.parse::<u32>().ok()) else {
            continue;
        };
        if !(MIN_UID..=MAX_UID).contains(&uid) {
            continue;
        }
        // A login shell is what separates a person from a service account that
        // happens to sit in the same uid range.
        if shell.ends_with("/nologin") || shell.ends_with("/false") || shell.is_empty() {
            continue;
        }
        if !uids.contains(&uid) {
            uids.push(uid);
        }
    }
    uids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curated(dir: &Path, name: &str, app_id: &str) -> PathBuf {
        let p = dir.join(format!("{name}.toml"));
        std::fs::write(
            &p,
            format!("[info]\napp_id = \"{app_id}\"\ntier = \"third-party\"\n"),
        )
        .unwrap();
        p
    }

    #[test]
    fn a_matched_package_is_written_for_every_uid() {
        let tmp = tempfile::tempdir().unwrap();
        let cur = tmp.path().join("curated");
        std::fs::create_dir_all(&cur).unwrap();
        let base = tmp.path().join("system");
        let profile = curated(&cur, "hello", "hello");

        let out = enroll_matched(&[("hello".to_string(), profile)], &[1000, 1001], &base);
        let Enrolled::Written { package, paths } = &out[0] else {
            panic!("expected a written enrolment, got {:?}", out[0]);
        };
        assert_eq!(package, "hello");
        // A .deb is machine-wide but a profile is per-uid, so one package
        // enrolment has to land once per human account or the second user runs
        // the app unconfined.
        assert_eq!(paths.len(), 2);
        for p in paths {
            assert!(p.is_file(), "{} was not written", p.display());
            assert!(std::fs::read_to_string(p).unwrap().contains("app_id = \"hello\""));
        }
    }

    #[test]
    fn the_curated_content_is_copied_verbatim() {
        // The curated profile is the reviewed grant. Rewriting it here would
        // create a second source of truth for what a package may do.
        let tmp = tempfile::tempdir().unwrap();
        let cur = tmp.path().join("curated");
        std::fs::create_dir_all(&cur).unwrap();
        let base = tmp.path().join("system");
        let body = "[info]\napp_id = \"vlc\"\ntier = \"third-party\"\n\n[network]\nallowed_domains = [\"example.org\"]\n";
        let profile = cur.join("vlc.toml");
        std::fs::write(&profile, body).unwrap();

        let out = enroll_matched(&[("vlc".to_string(), profile)], &[1000], &base);
        let Enrolled::Written { paths, .. } = &out[0] else {
            panic!("expected a written enrolment");
        };
        assert_eq!(std::fs::read_to_string(&paths[0]).unwrap(), body);
    }

    #[test]
    fn one_bad_package_does_not_stop_the_others() {
        // An apt hook exiting non-zero can abort the package operation. Failing
        // to confine one package must not make the machine unable to install
        // software, so a failure is reported and the run continues.
        let tmp = tempfile::tempdir().unwrap();
        let cur = tmp.path().join("curated");
        std::fs::create_dir_all(&cur).unwrap();
        let base = tmp.path().join("system");
        let good = curated(&cur, "hello", "hello");
        let missing = cur.join("does-not-exist.toml");

        let out = enroll_matched(
            &[
                ("missing".to_string(), missing),
                ("hello".to_string(), good),
            ],
            &[1000],
            &base,
        );
        assert!(matches!(out[0], Enrolled::Failed { .. }));
        assert!(matches!(out[1], Enrolled::Written { .. }));
    }

    #[test]
    fn a_malformed_curated_profile_is_refused_not_persisted() {
        // write_profile_in re-validates, so a curated file that is not a profile
        // cannot reach the system tier and be read as one later.
        let tmp = tempfile::tempdir().unwrap();
        let cur = tmp.path().join("curated");
        std::fs::create_dir_all(&cur).unwrap();
        let base = tmp.path().join("system");
        let bad = cur.join("junk.toml");
        std::fs::write(&bad, "this is not = valid [ toml").unwrap();

        let out = enroll_matched(&[("junk".to_string(), bad)], &[1000], &base);
        assert!(matches!(out[0], Enrolled::Failed { .. }));
        assert!(
            !base.join("1000").join("junk.toml").exists(),
            "a refused profile must leave nothing behind"
        );
    }

    #[test]
    fn only_human_accounts_are_enrolled() {
        let passwd = "\
root:x:0:0:root:/root:/bin/bash
daemon:x:1:1:daemon:/usr/sbin:/usr/sbin/nologin
systemd-network:x:998:998::/:/usr/sbin/nologin
tim:x:1000:1000:Tim:/home/tim:/bin/zsh
alice:x:1001:1001::/home/alice:/bin/bash
builder:x:1002:1002::/var/lib/builder:/bin/false
nobody:x:65534:65534:nobody:/nonexistent:/usr/sbin/nologin
";
        // root and system accounts do not launch desktop apps; a profile written
        // for them is a dead file that later reads as an enrolled app.
        assert_eq!(human_uids(passwd), vec![1000, 1001]);
    }

    #[test]
    fn a_malformed_passwd_line_is_skipped_not_fatal() {
        // /etc/passwd is not ours; one unparseable line must not cost the
        // enrolment every other account on the machine.
        let passwd = "garbage\n\ntim:x:1000:1000:Tim:/home/tim:/bin/zsh\nbroken:x:notanumber:0::/:/bin/sh\n";
        assert_eq!(human_uids(passwd), vec![1000]);
    }
}
