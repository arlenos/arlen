//! `arlen-run` - the confined app launcher.
//!
//! A fork-exec binary (not a daemon) that the shell's `launch_app` execs with an
//! app identity and a program to run. `arlen-run` loads the app's permission
//! profile, then (in later commits) applies Landlock + seccomp + a per-command
//! cgroup and the egress filter and spawns the app under bwrap, becoming its
//! long-lived confined parent. It replaces the unconfined `sh -c` launch path.
//!
//! Fail-closed is the whole point: any setup failure - a missing/unparsable
//! profile, a confinement-setup error, an egress-filter failure - means the app
//! NEVER starts. There is no "run with reduced confinement" path; a missing
//! profile is a deny, not a default-open.
//!
//! This commit is the CLI surface: argv parsing, app-id validation, the exit-code
//! contract, the non-Linux stub, and a profile load that prints the program it
//! would run. The confinement layers land in later commits.

use std::path::PathBuf;
use std::process::ExitCode;

mod profile;

/// The fail-closed exit-code contract. Any setup failure means the app never
/// starts; otherwise the app's own exit code is propagated.
pub mod exit {
    /// The app exited successfully (or, pre-confinement, the dry run succeeded).
    pub const OK: u8 = 0;
    /// Malformed argv or an invalid app-id.
    pub const BAD_ARGS: u8 = 64;
    /// The profile was missing or unparsable - DENY, never run unconfined.
    pub const PROFILE: u8 = 65;
    /// Landlock/seccomp/cgroup/bwrap setup failed - never spawn.
    pub const CONFINE_SETUP: u8 = 66;
    /// The egress filter could not be installed for a `FilteredHosts` profile.
    pub const EGRESS: u8 = 67;
    /// bwrap failed to exec the app.
    pub const SPAWN: u8 = 68;
    /// Built for a non-Linux target, where confinement is unavailable.
    pub const NOT_LINUX: u8 = 2;
}

/// Whether `app_id` is a valid reverse-DNS app id safe to put into a profile path
/// AND a cgroup name: a non-empty lowercase `[a-z0-9._-]` id with at least one dot,
/// no `..`, and no leading/trailing dot. It lands in both a filesystem path and a
/// cgroup leaf name, so it is validated strictly before either.
fn valid_app_id(app_id: &str) -> bool {
    !app_id.is_empty()
        && app_id.contains('.')
        && !app_id.starts_with('.')
        && !app_id.ends_with('.')
        && !app_id.contains("..")
        && app_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// The parsed launch request.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    /// The reverse-DNS app id (validated).
    app_id: String,
    /// Optional override for the directory `{app_id}.toml` is read from.
    profile_root: Option<PathBuf>,
    /// The program and its argv (everything after `--`).
    program: Vec<String>,
}

/// Parse `arlen-run --app-id <id> [--profile-root <dir>] -- <program> [args...]`
/// from the argument list (excluding the binary name). Returns the parsed request,
/// or the exit code to fail with: an unknown flag, a missing/invalid `--app-id`, a
/// missing `--`, or an empty program is `BAD_ARGS`.
fn parse_args(args: &[String]) -> Result<Args, u8> {
    let mut app_id: Option<String> = None;
    let mut profile_root: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--app-id" => {
                let value = args.get(i + 1).ok_or(exit::BAD_ARGS)?;
                app_id = Some(value.clone());
                i += 2;
            }
            "--profile-root" => {
                let value = args.get(i + 1).ok_or(exit::BAD_ARGS)?;
                profile_root = Some(PathBuf::from(value));
                i += 2;
            }
            "--" => {
                let program: Vec<String> = args[i + 1..].to_vec();
                if program.is_empty() {
                    return Err(exit::BAD_ARGS);
                }
                let app_id = app_id.ok_or(exit::BAD_ARGS)?;
                if !valid_app_id(&app_id) {
                    return Err(exit::BAD_ARGS);
                }
                return Ok(Args {
                    app_id,
                    profile_root,
                    program,
                });
            }
            _ => return Err(exit::BAD_ARGS),
        }
    }
    // No `--` separator: there is no program to run.
    Err(exit::BAD_ARGS)
}

#[cfg(target_os = "linux")]
fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(code) => return ExitCode::from(code),
    };

    // Load the app's permission profile. A missing or unparsable profile is a DENY
    // (the confined launcher must never run an app it cannot scope), not a default.
    let profile = match &args.profile_root {
        Some(root) => {
            let path = root.join(format!("{}.toml", args.app_id));
            arlen_permissions::load_profile_from(&path, &args.app_id)
        }
        None => arlen_permissions::load_profile(&args.app_id),
    };
    let profile = match profile {
        Ok(p) => p,
        Err(e) => {
            eprintln!("arlen-run: profile for {}: {e}", args.app_id);
            return ExitCode::from(exit::PROFILE);
        }
    };

    // Derive the confiner inputs (the writable set + the network policy) from the
    // profile. The full bwrap spawn + Landlock + seccomp + cgroup + egress land in
    // later commits; for now report what the confinement would be.
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let user_dirs = profile::UserDirs {
        documents: dirs::document_dir().unwrap_or_else(|| home.join("Documents")),
        downloads: dirs::download_dir().unwrap_or_else(|| home.join("Downloads")),
        pictures: dirs::picture_dir().unwrap_or_else(|| home.join("Pictures")),
        music: dirs::audio_dir().unwrap_or_else(|| home.join("Music")),
        videos: dirs::video_dir().unwrap_or_else(|| home.join("Videos")),
    };
    let inputs = profile::confinement_inputs(
        &profile.filesystem,
        &profile.network,
        &args.app_id,
        &home,
        &user_dirs,
    );
    println!(
        "arlen-run: would launch {} confined as {} (network {:?}, {} writable dirs)",
        args.program.join(" "),
        args.app_id,
        inputs.network,
        inputs.app_dirs.len()
    );
    ExitCode::from(exit::OK)
}

#[cfg(not(target_os = "linux"))]
fn main() -> ExitCode {
    eprintln!("arlen-run: confinement is only available on Linux");
    ExitCode::from(exit::NOT_LINUX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn valid_app_ids() {
        assert!(valid_app_id("com.example.notes"));
        assert!(valid_app_id("org.kde.app2"));
        assert!(valid_app_id("a.b"));
    }

    #[test]
    fn invalid_app_ids() {
        for bad in [
            "",
            "noseparator",       // no dot
            "UPPER.case",        // uppercase
            ".leading",          // leading dot
            "trailing.",         // trailing dot
            "a..b",              // double dot
            "a/b.c",             // separator
            "a b.c",             // space
        ] {
            assert!(!valid_app_id(bad), "{bad:?} must be invalid");
        }
    }

    #[test]
    fn parses_a_full_invocation() {
        let a = parse_args(&args(&[
            "--app-id",
            "com.example.app",
            "--",
            "/usr/bin/foo",
            "--flag",
            "x",
        ]))
        .unwrap();
        assert_eq!(a.app_id, "com.example.app");
        assert_eq!(a.profile_root, None);
        assert_eq!(a.program, ["/usr/bin/foo", "--flag", "x"]);
    }

    #[test]
    fn parses_a_profile_root() {
        let a = parse_args(&args(&[
            "--profile-root",
            "/var/lib/arlen/permissions/1000",
            "--app-id",
            "com.a.b",
            "--",
            "prog",
        ]))
        .unwrap();
        assert_eq!(
            a.profile_root,
            Some(PathBuf::from("/var/lib/arlen/permissions/1000"))
        );
        assert_eq!(a.program, ["prog"]);
    }

    #[test]
    fn rejects_a_missing_app_id() {
        assert_eq!(parse_args(&args(&["--", "prog"])), Err(exit::BAD_ARGS));
    }

    #[test]
    fn rejects_an_invalid_app_id() {
        assert_eq!(
            parse_args(&args(&["--app-id", "no-dot", "--", "prog"])),
            Err(exit::BAD_ARGS)
        );
    }

    #[test]
    fn rejects_a_missing_separator_or_empty_program() {
        assert_eq!(
            parse_args(&args(&["--app-id", "com.a.b"])),
            Err(exit::BAD_ARGS)
        );
        assert_eq!(
            parse_args(&args(&["--app-id", "com.a.b", "--"])),
            Err(exit::BAD_ARGS)
        );
    }

    #[test]
    fn rejects_an_unknown_flag() {
        assert_eq!(
            parse_args(&args(&["--bogus", "x", "--app-id", "com.a.b", "--", "prog"])),
            Err(exit::BAD_ARGS)
        );
    }
}
