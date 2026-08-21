//! Building the command line that starts a program in a bottle.
//!
//! Lifted out of the example that proved it, because the process that will own
//! this is not decided yet and the assembly should not be decided with it. Two
//! shapes are open: the window spawns the program itself, or a bottle daemon
//! does. The daemon is what `wine-proton-plan.md` describes, and it is the better
//! answer for a reason the plan does not give: if the window is itself confined
//! by `arlen-run`, it is a bwrap child trying to spawn a bwrap child, and the
//! nesting is a real constraint rather than a preference. Either way the argv is
//! the same, and it belongs here where it can be tested without spawning
//! anything.
//!
//! The refusals are the interesting part. A launch is stopped BEFORE it starts
//! when the drive letters promise more than the sandbox delivers, because a
//! program that meets a drive it cannot open produces no error anyone can act on.

use std::collections::BTreeMap;
use std::path::Path;

use arlen_confiner::merged_usr_compat_roots;

use crate::bottle::{bottle_run, unmet_drives, Bottle, BottleError, UnmetDrive};
use crate::plumbing::plumbing_binds;

/// Why a launch was refused.
#[derive(Debug)]
pub enum LaunchError {
    /// The bottle could not be turned into a confinement.
    Bottle(BottleError),
    /// The drive table promises reach the sandbox does not give. Refused rather
    /// than started: the program would see a drive it cannot open.
    UnmetDrives(Vec<UnmetDrive>),
    /// No program was named.
    NoProgram,
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::Bottle(e) => write!(f, "{e}"),
            LaunchError::UnmetDrives(u) => {
                write!(f, "{} drive(s) promise more than the sandbox gives:", u.len())?;
                for d in u {
                    write!(f, " {}: {:?} promised, {:?} given", d.letter, d.promised, d.actual)?;
                }
                Ok(())
            }
            LaunchError::NoProgram => write!(f, "no program was named"),
        }
    }
}

impl std::error::Error for LaunchError {}

/// The environment a bottle's program runs with.
///
/// Explicit rather than inherited: the confinement clears the environment, so
/// anything not named here is absent, and that is the point. A Windows program
/// has no business reading the session's variables.
pub fn launch_env(bottle: &Bottle, display: Option<&str>) -> BTreeMap<String, String> {
    let prefix = bottle.prefix_root.display().to_string();
    let mut env = BTreeMap::new();
    env.insert("WINEPREFIX".into(), prefix.clone());
    // The prefix is the bottle's home. Wine writes a cache beside it, and nothing
    // of the person's is reachable to write into anyway.
    env.insert("HOME".into(), prefix);
    env.insert("PATH".into(), "/usr/bin".into());
    env.insert("WINEDEBUG".into(), "-all".into());
    // Mono and Gecko are offered through a modal, and a bottle has no display to
    // show one on at the moment it would appear. Without this the first launch
    // waits for an answer that cannot arrive: measured at over ten minutes,
    // against ten seconds with it set. A bottle that needs .NET gets Mono
    // installed deliberately.
    env.insert("WINEDLLOVERRIDES".into(), "mscoree,mshtml=".into());
    if let Some(d) = display {
        env.insert("DISPLAY".into(), d.to_string());
    }
    env
}

/// The full `bwrap` argument list for running `program` in `bottle`.
///
/// `exists` decides which plumbing is on this host, injected so the list can be
/// built for a machine that is not this one.
pub fn launch_argv(
    bottle: &Bottle,
    usr: &Path,
    runtime_dir: &Path,
    display: Option<&str>,
    program: &[String],
    exists: impl Fn(&Path) -> bool,
) -> Result<Vec<String>, LaunchError> {
    if program.is_empty() {
        return Err(LaunchError::NoProgram);
    }
    let binds = plumbing_binds(&bottle.plumbing, runtime_dir, &exists);
    let run = bottle_run(bottle, usr, launch_env(bottle, display), binds)
        .map_err(LaunchError::Bottle)?;
    let unmet = unmet_drives(&run.confinement, &run.drives);
    if !unmet.is_empty() {
        return Err(LaunchError::UnmetDrives(unmet));
    }

    let mut argv = run.confinement.bwrap_args();
    // The root-level compatibility paths, if this host has them. A merged-`/usr`
    // machine reaches its libraries through `/lib64` and friends, and a bind of a
    // path that is not there fails the whole launch.
    for root in merged_usr_compat_roots() {
        if exists(Path::new(&root)) {
            argv.push("--ro-bind".into());
            argv.push(root.clone());
            argv.push(root);
        }
    }
    argv.push("--".into());
    argv.push("/usr/bin/wine".into());
    argv.extend(program.iter().cloned());
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bottle::Egress;
    use crate::plumbing::{Display, Plumbing};
    use crate::{Access, PathGrant};
    use std::path::PathBuf;

    fn bottle() -> Bottle {
        Bottle {
            id: "notepad".into(),
            prefix_root: PathBuf::from("/data/bottles/notepad/pfx"),
            grants: vec![PathGrant {
                host: PathBuf::from("/home/u/Projects"),
                access: Access::ReadWrite,
            }],
            egress: Egress::None,
            plumbing: Plumbing { display: Display::X11, gpu: false, fonts: true },
        }
    }

    fn all(_: &Path) -> bool {
        true
    }

    #[test]
    fn the_program_comes_last_after_the_separator() {
        let argv = launch_argv(
            &bottle(),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            Some(":0"),
            &["notepad".into(), "D:\\a.txt".into()],
            all,
        )
        .unwrap();
        let sep = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(&argv[sep + 1..], ["/usr/bin/wine", "notepad", "D:\\a.txt"]);
    }

    #[test]
    fn nothing_of_the_session_is_carried_in() {
        let env = launch_env(&bottle(), None);
        assert_eq!(env.get("WINEPREFIX").unwrap(), "/data/bottles/notepad/pfx");
        assert_eq!(env.get("HOME").unwrap(), "/data/bottles/notepad/pfx");
        assert!(!env.contains_key("DISPLAY"), "a bottle that draws nothing is told nothing");
        assert_eq!(env.get("WINEDLLOVERRIDES").unwrap(), "mscoree,mshtml=");
    }

    #[test]
    fn a_drive_the_sandbox_does_not_back_stops_the_launch() {
        // The grant is under /proc, which the confinement mounts as its own
        // private procfs, so the bind is masked and the letter would point at
        // nothing. Refused rather than started.
        let mut b = bottle();
        b.grants = vec![PathGrant {
            host: PathBuf::from("/proc/self/fd"),
            access: Access::ReadWrite,
        }];
        let err = launch_argv(
            &b,
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            None,
            &["notepad".into()],
            all,
        );
        assert!(matches!(err, Err(LaunchError::UnmetDrives(_))), "{err:?}");
    }

    #[test]
    fn a_launch_with_no_program_is_refused() {
        let err = launch_argv(
            &bottle(),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            None,
            &[],
            all,
        );
        assert!(matches!(err, Err(LaunchError::NoProgram)));
    }

    #[test]
    fn a_compat_root_this_host_lacks_is_not_bound() {
        let argv = launch_argv(
            &bottle(),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            None,
            &["notepad".into()],
            |p| p != Path::new("/lib64"),
        )
        .unwrap();
        assert!(!argv.iter().any(|a| a == "/lib64"), "bwrap fails the whole launch on a missing source");
    }
}
