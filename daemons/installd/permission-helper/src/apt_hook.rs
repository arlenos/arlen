//! The `.deb` apt-enroll hook parser (app-enrollment §E4).
//!
//! apt streams a `DPkg::Pre-Install-Pkgs` Version 2/3 block to the configured
//! hook on stdin: a `VERSION N` line, a config section (`APT::Key=value` lines),
//! a blank line, then one line per package change. This parses that stream into
//! the package changes installd matches to a curated profile by BARE NAME (the
//! `.deb` package name is exactly the key of the curated starting profiles).
//!
//! The grammar here was CAPTURED FROM A REAL apt run in a `debian:trixie`
//! container (not built against the doc alone), VERSION 3:
//!
//! ```text
//! VERSION 3
//! APT::Architecture=amd64
//! ... (config section) ...
//!                                          <- blank line
//! hello 2.10-5 amd64 none = 2.10-5 amd64 none /var/cache/apt/archives/hello_2.10-5_amd64.deb
//! hello 2.10-5 amd64 none = 2.10-5 amd64 none **CONFIGURE**
//! ```
//!
//! A v3 package line is nine whitespace-separated fields:
//! `name old-ver old-arch old-multiarch compare new-ver new-arch new-multiarch action`.
//! A v2 line drops the arch/multiarch fields:
//! `name old-ver compare new-ver action`. The action is the last field: a `.deb`
//! path (the unpack/install), or `**CONFIGURE**` / `**REMOVE**` / `**PURGE**`.

use std::fmt;
use std::path::{Path, PathBuf};

/// What apt is doing to the package in this change line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Unpack/install the `.deb` at this path (the enroll trigger).
    Install(String),
    /// Configure an already-unpacked package (files have landed).
    Configure,
    /// Remove the package (unenroll).
    Remove,
    /// Purge the package and its config (unenroll).
    Purge,
}

/// One package change from the Pre-Install-Pkgs stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageChange {
    /// The bare package name - the curated-profile key.
    pub name: String,
    /// The currently-installed version, or `None` for a fresh install (`-`).
    pub old_version: Option<String>,
    /// The version being installed/configured, or `None` if absent (`-`).
    pub new_version: Option<String>,
    /// What apt is doing.
    pub action: Action,
}

/// Why a Pre-Install-Pkgs stream could not be parsed. Fail-closed: a malformed
/// stream yields no enrollments rather than a guessed one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AptHookError {
    /// The stream did not start with a recognised `VERSION 2`/`VERSION 3` line.
    BadVersion,
    /// A package line did not have the field count its version dictates.
    MalformedLine(String),
}

impl fmt::Display for AptHookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AptHookError::BadVersion => write!(f, "stream is not a Pre-Install-Pkgs v2/v3 block"),
            AptHookError::MalformedLine(l) => write!(f, "malformed package line: {l}"),
        }
    }
}

fn version_or_none(field: &str) -> Option<String> {
    if field == "-" {
        None
    } else {
        Some(field.to_string())
    }
}

fn action_from(last: &str) -> Action {
    match last {
        "**CONFIGURE**" => Action::Configure,
        "**REMOVE**" => Action::Remove,
        "**PURGE**" => Action::Purge,
        path => Action::Install(path.to_string()),
    }
}

/// Parse a `DPkg::Pre-Install-Pkgs` Version 2 or 3 stream into its package
/// changes. Fail-closed: an unrecognised version errors, and a package line with
/// the wrong field count errors rather than being guessed.
pub fn parse_pre_install_pkgs(stream: &str) -> Result<Vec<PackageChange>, AptHookError> {
    let mut lines = stream.lines();
    let version = match lines.next().map(str::trim) {
        Some("VERSION 3") => 3,
        Some("VERSION 2") => 2,
        _ => return Err(AptHookError::BadVersion),
    };

    // The config section runs until the first blank line; package lines follow.
    let mut in_packages = false;
    let mut changes = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if !in_packages {
            if line.is_empty() {
                in_packages = true;
            }
            continue;
        }
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split_whitespace().collect();
        // v3: name old-ver old-arch old-multiarch cmp new-ver new-arch new-multiarch action
        // v2: name old-ver cmp new-ver action
        let (name, old, new, action) = match (version, fields.as_slice()) {
            (3, [name, old, _oa, _om, _cmp, new, _na, _nm, action]) => (*name, *old, *new, *action),
            (2, [name, old, _cmp, new, action]) => (*name, *old, *new, *action),
            _ => return Err(AptHookError::MalformedLine(line.to_string())),
        };
        changes.push(PackageChange {
            name: name.to_string(),
            old_version: version_or_none(old),
            new_version: version_or_none(new),
            action: action_from(action),
        });
    }
    Ok(changes)
}

/// The package names to ENROLL from a parsed stream: those whose action is the
/// `.deb` unpack/install (a fresh install or an upgrade), deduped in order. A
/// `**CONFIGURE**` line for the same package is the post-unpack step, not a
/// second enrollment; `**REMOVE**`/`**PURGE**` are un-enrollments handled
/// elsewhere.
pub fn packages_to_enroll(changes: &[PackageChange]) -> Vec<String> {
    let mut names = Vec::new();
    for c in changes {
        if matches!(c.action, Action::Install(_)) && !names.contains(&c.name) {
            names.push(c.name.clone());
        }
    }
    names
}

/// For each package a Pre-Install-Pkgs stream would enroll, the curated starting
/// profile that covers it (if one exists under `curated_dir`). This is the MATCH
/// half of the `.deb` enroll: parse the stream, take the install-action packages,
/// and pair each with `curated_dir/<name>.toml` (the `.deb` bare name is exactly
/// the curated-profile key). A package with no curated profile is omitted (it is
/// left to the learning-mode fallback, §E9). The WRITE target (system vs user
/// profile, per-uid) is a deploy decision the caller makes; this function does no
/// I/O beyond checking each candidate profile exists.
pub fn match_enrollments(
    stream: &str,
    curated_dir: &Path,
) -> Result<Vec<(String, PathBuf)>, AptHookError> {
    let changes = parse_pre_install_pkgs(stream)?;
    let mut matched = Vec::new();
    for name in packages_to_enroll(&changes) {
        let profile = curated_dir.join(format!("{name}.toml"));
        if profile.is_file() {
            matched.push((name, profile));
        }
    }
    Ok(matched)
}

/// NOT YET WIRED: the profile-writing half of the enrolment runs (see
/// `apt_enroll`), but nothing yet shells out to `dpkg -L` and rewrites the
/// `.desktop` entries, so an enrolled package has a profile and still launches
/// unconfined. Kept and tested here because it is the next stage of the same
/// hook, not speculative surface.
///
/// The installable artifacts of a `.deb`, classified from `dpkg -L <pkg>` output:
/// the executables (which the enrollment confines) and the `.desktop` entries
/// (whose `Exec=` the enrollment rewrites to launch through `arlen-run`).
/// Best-effort: `dpkg -L` lists only packaged files, so it misses files created
/// by maintainer scripts or `update-alternatives`.
#[allow(dead_code)] // the .desktop-rewrite stage; see the note above
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackageFiles {
    /// Executables shipped directly in a system bin directory.
    pub binaries: Vec<PathBuf>,
    /// `.desktop` application entries.
    pub desktop_entries: Vec<PathBuf>,
}

/// The system bin directories a `.deb`'s executables land in (trailing slash so a
/// bare directory line does not match).
const BIN_DIRS: &[&str] = &[
    "/usr/bin/",
    "/usr/sbin/",
    "/bin/",
    "/sbin/",
    "/usr/games/",
];

/// Classify the files a package ships, from `dpkg -L <pkg>` output (one absolute
/// path per line, files and directories intermixed). A binary is a file directly
/// in a system bin directory; a `.desktop` entry is under the applications dir.
#[allow(dead_code)] // the .desktop-rewrite stage; see PackageFiles
pub fn classify_package_files(dpkg_l: &str) -> PackageFiles {
    let mut files = PackageFiles::default();
    for raw in dpkg_l.lines() {
        let path = raw.trim();
        if path.is_empty() || path == "/." {
            continue;
        }
        if path.starts_with("/usr/share/applications/") && path.ends_with(".desktop") {
            files.desktop_entries.push(PathBuf::from(path));
            continue;
        }
        if let Some(dir) = BIN_DIRS.iter().find(|d| path.starts_with(**d)) {
            let rest = &path[dir.len()..];
            // Only an executable file DIRECTLY in the bin dir (a non-empty
            // remainder with no further `/`), not a nested path or the dir itself.
            if !rest.is_empty() && !rest.contains('/') {
                files.binaries.push(PathBuf::from(path));
            }
        }
    }
    files.binaries.sort();
    files.desktop_entries.sort();
    files
}

/// Rewrite a `.desktop` file's `Exec=` line(s) to launch the app confined through
/// `arlen-run`: `Exec=<cmd>` becomes `Exec=arlen-run --app-id <app_id> -- <cmd>`.
/// The original command (including any `.desktop` field codes like `%U`/`%f`,
/// which the launcher substitutes before exec) is preserved verbatim after the
/// `--` separator, so `arlen-run` passes it to the app. Every other line - the
/// group headers, `TryExec`, `Name`, comments - is preserved byte-for-byte, and
/// the main `Exec` plus any `[Desktop Action ...]` `Exec` lines are all rewritten.
/// Idempotent: an `Exec` already routed through `arlen-run` is left untouched.
///
/// `app_id` must be a validated package/app id (no whitespace) - it is the `.deb`
/// bare name the enrollment resolved, so it cannot break the `Exec` line.
#[allow(dead_code)] // the .desktop-rewrite stage; see PackageFiles
pub fn rewrite_desktop_exec(content: &str, app_id: &str) -> String {
    let mut out = String::with_capacity(content.len() + 48);
    for line in content.lines() {
        match line.strip_prefix("Exec=") {
            Some(value) if !value.trim().is_empty() && !value.trim_start().starts_with("arlen-run ") => {
                out.push_str("Exec=arlen-run --app-id ");
                out.push_str(app_id);
                out.push_str(" -- ");
                out.push_str(value);
            }
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact stream captured from a real `apt install --reinstall hello` in a
    /// debian:trixie container (VERSION 3).
    const REAL_V3: &str = "VERSION 3\n\
APT::Architecture=amd64\n\
APT::Install-Recommends=1\n\
APT::Sandbox::User=_apt\n\
\n\
hello 2.10-5 amd64 none = 2.10-5 amd64 none /var/cache/apt/archives/hello_2.10-5_amd64.deb\n\
hello 2.10-5 amd64 none = 2.10-5 amd64 none **CONFIGURE**\n";

    #[test]
    fn parses_the_real_v3_stream() {
        let changes = parse_pre_install_pkgs(REAL_V3).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].name, "hello");
        assert_eq!(changes[0].old_version.as_deref(), Some("2.10-5"));
        assert_eq!(changes[0].new_version.as_deref(), Some("2.10-5"));
        assert_eq!(
            changes[0].action,
            Action::Install("/var/cache/apt/archives/hello_2.10-5_amd64.deb".into())
        );
        assert_eq!(changes[1].action, Action::Configure);
        // The unpack enrolls once; the CONFIGURE line is not a second enroll.
        assert_eq!(packages_to_enroll(&changes), vec!["hello".to_string()]);
    }

    #[test]
    fn a_fresh_install_has_no_old_version() {
        let s = "VERSION 3\nAPT::Architecture=amd64\n\n\
                 gimp - none none < 2.10.38 amd64 none /var/cache/apt/archives/gimp.deb\n";
        let changes = parse_pre_install_pkgs(s).unwrap();
        assert_eq!(changes[0].name, "gimp");
        assert_eq!(changes[0].old_version, None); // `-` = not installed
        assert_eq!(changes[0].new_version.as_deref(), Some("2.10.38"));
    }

    #[test]
    fn parses_v2_without_arch_fields() {
        let s = "VERSION 2\nAPT::Architecture=amd64\n\n\
                 hello 2.10-5 = 2.10-5 /var/cache/apt/archives/hello.deb\n\
                 hello 2.10-5 = 2.10-5 **REMOVE**\n";
        let changes = parse_pre_install_pkgs(s).unwrap();
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].name, "hello");
        assert_eq!(changes[1].action, Action::Remove);
        assert_eq!(packages_to_enroll(&changes), vec!["hello".to_string()]);
    }

    #[test]
    fn match_enrollments_pairs_a_package_with_its_curated_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // A curated profile exists for `hello`, but not for `unknownpkg`.
        std::fs::write(dir.join("hello.toml"), "[info]\napp_id = \"hello\"\n").unwrap();
        let stream = "VERSION 3\nAPT::Architecture=amd64\n\n\
            hello 2.10-5 amd64 none < 2.10-5 amd64 none /var/cache/apt/archives/hello.deb\n\
            unknownpkg - none none < 1.0 amd64 none /var/cache/apt/archives/unknownpkg.deb\n";
        let matched = match_enrollments(stream, dir).unwrap();
        // Only the package WITH a curated profile is matched; the unknown one is
        // left to the learning-mode fallback.
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].0, "hello");
        assert_eq!(matched[0].1, dir.join("hello.toml"));
    }

    #[test]
    fn classify_finds_the_binary_from_real_dpkg_l() {
        // A trimmed slice of the real `dpkg -L hello` output (debian:trixie): the
        // binary is the only file directly in a bin dir.
        let dpkg_l = "/.\n/usr\n/usr/bin\n/usr/bin/hello\n/usr/share\n\
                      /usr/share/doc/hello/copyright\n/usr/share/info/hello.info.gz\n";
        let f = classify_package_files(dpkg_l);
        assert_eq!(f.binaries, vec![PathBuf::from("/usr/bin/hello")]);
        assert!(f.desktop_entries.is_empty());
    }

    #[test]
    fn classify_separates_binaries_and_desktop_ignoring_dirs_and_nested_paths() {
        let dpkg_l = "/usr/bin\n/usr/bin/foo\n/usr/sbin/food\n/usr/lib/foo/helper\n\
                      /usr/bin/nested/deep\n/usr/share/applications\n\
                      /usr/share/applications/foo.desktop\n/usr/share/doc/foo/README\n";
        let f = classify_package_files(dpkg_l);
        // Only top-level bin files; the libexec helper + nested bin path are not
        // binaries, and the bare dir lines are ignored.
        assert_eq!(
            f.binaries,
            vec![
                PathBuf::from("/usr/bin/foo"),
                PathBuf::from("/usr/sbin/food")
            ]
        );
        assert_eq!(
            f.desktop_entries,
            vec![PathBuf::from("/usr/share/applications/foo.desktop")]
        );
    }

    #[test]
    fn rewrite_confines_every_exec_preserving_field_codes_and_other_keys() {
        let desktop = "[Desktop Entry]\nName=Foo\nTryExec=/usr/bin/foo\n\
                       Exec=/usr/bin/foo %U\nIcon=foo\n\n\
                       [Desktop Action new]\nName=New Window\nExec=/usr/bin/foo --new %f\n";
        let out = rewrite_desktop_exec(desktop, "com.foo");
        // Both the main Exec and the action Exec are confined, field codes kept.
        assert!(out.contains("Exec=arlen-run --app-id com.foo -- /usr/bin/foo %U\n"));
        assert!(out.contains("Exec=arlen-run --app-id com.foo -- /usr/bin/foo --new %f\n"));
        // TryExec (a check, not a launch), names and group headers are untouched.
        assert!(out.contains("TryExec=/usr/bin/foo\n"));
        assert!(out.contains("[Desktop Action new]\n"));
        assert!(out.contains("Name=Foo\n"));
    }

    #[test]
    fn rewrite_is_idempotent_and_skips_an_empty_exec() {
        let already = "[Desktop Entry]\nExec=arlen-run --app-id com.foo -- /usr/bin/foo %U\n";
        assert_eq!(rewrite_desktop_exec(already, "com.foo"), already);
        let empty = "[Desktop Entry]\nExec=\n";
        assert_eq!(rewrite_desktop_exec(empty, "com.foo"), empty);
    }

    #[test]
    fn a_bad_version_or_malformed_line_fails_closed() {
        assert_eq!(parse_pre_install_pkgs("VERSION 1\n\n"), Err(AptHookError::BadVersion));
        assert_eq!(parse_pre_install_pkgs("junk"), Err(AptHookError::BadVersion));
        let s = "VERSION 3\nAPT::Architecture=amd64\n\ntoo few fields here\n";
        assert!(matches!(
            parse_pre_install_pkgs(s),
            Err(AptHookError::MalformedLine(_))
        ));
    }
}
