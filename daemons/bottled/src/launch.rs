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
use std::path::{Path, PathBuf};

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
    /// There is no Wine on this machine, so nothing could run the program.
    ///
    /// Refused for the same reason as an unmet drive: without this the argv is
    /// built, bwrap starts, and the failure surfaces from inside the sandbox as
    /// a missing file - which names neither Wine nor the fact that this system
    /// has none.
    NoRuntime(PathBuf),
}

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchError::Bottle(e) => write!(f, "{e}"),
            LaunchError::UnmetDrives(u) => {
                write!(
                    f,
                    "{} drive(s) promise more than the sandbox gives:",
                    u.len()
                )?;
                for d in u {
                    write!(
                        f,
                        " {}: {:?} promised, {:?} given",
                        d.letter, d.promised, d.actual
                    )?;
                }
                Ok(())
            }
            LaunchError::NoProgram => write!(f, "no program was named"),
            LaunchError::NoRuntime(p) => {
                write!(f, "there is no Wine at {} on this machine", p.display())
            }
        }
    }
}

impl std::error::Error for LaunchError {}

/// Where Wine is, when the machine has one.
///
/// One string rather than two: the check below and the argv have to name the
/// same binary, and they did not have to for long before somebody moved one.
pub const WINE: &str = "/usr/bin/wine";

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
/// A Wine command, followed by a wait for the registry to be written.
///
/// **The wait is not tidiness, it is the difference between the step happening
/// and not happening.** A Wine process hands its registry changes to wineserver
/// and exits; the server writes them to `user.reg` and `system.reg` when IT
/// exits. `bwrap --die-with-parent --unshare-pid` tears the sandbox down the
/// moment the program it was given exits, so the server dies first and the
/// changes are gone - with every exit code still zero.
///
/// Measured on 7 September, in both places it applies. A confined `wineboot -u`
/// left a prefix holding `dosdevices` and `drive_c` and NO registry at all; the
/// same command followed by `wineserver -w` left a 3.8 MB one. A confined
/// registry import reported success and changed nothing.
///
/// **Every Wine step uses this, including a program launch, and the reasoning
/// that said otherwise was wrong.** I wrote at first that a launch does not need
/// it because the app holds the server open itself - true while the app runs,
/// and beside the point: the loss happens when the app EXITS. A Windows program
/// that saves its settings to `HKCU` hands them to the server and quits, the
/// sandbox closes, and the settings are gone, so the app opens with defaults
/// every single time. Measured with the same probe as the other two: a confined
/// `reg add` lands nothing without the wait and lands with it.
///
/// What the wait costs a launch is that the sandbox outlives the app by the few
/// seconds the server takes to finish. What it buys is that anything the app
/// wrote is still there next time.
///
/// The shell is `/usr/bin/sh` rather than `/bin/sh` because `/usr` is always
/// bound and `/bin` only when the host has it as a real directory. Nothing is
/// interpolated into the script but the caller's fixed argument list.
pub fn settled_wine_argv(
    bottle: &Bottle,
    usr: &Path,
    runtime_dir: &Path,
    display: Option<&str>,
    wine_args: &[String],
    exists: impl Fn(&Path) -> bool,
) -> Result<Vec<String>, LaunchError> {
    let script = format!("{WINE} {}; {WINESERVER} -w", wine_args.join(" "));
    confined_argv(
        bottle,
        usr,
        runtime_dir,
        display,
        SHELL,
        &["-c".to_string(), script],
        exists,
    )
}

/// The shell that holds the sandbox open for the wait above.
pub const SHELL: &str = "/usr/bin/sh";

/// The process that actually writes a prefix's registry to disk.
pub const WINESERVER: &str = "/usr/bin/wineserver";

/// The same confinement around any executable inside it.
///
/// Split out because one step needs a command SEQUENCE rather than a program: a
/// registry import has to outlive `regedit` until wineserver writes the change
/// out, and killing the sandbox the moment `regedit` exits loses it silently -
/// measured, not guessed. Everything about the sandbox is identical; only what
/// is exec'd inside it differs, so the two cannot drift apart.
///
/// The Wine check stays here rather than moving to the caller: nothing in a
/// bottle runs without Wine, whatever the executable is, since the executable is
/// only ever a way of starting one.
pub fn confined_argv(
    bottle: &Bottle,
    usr: &Path,
    runtime_dir: &Path,
    display: Option<&str>,
    exec: &str,
    program: &[String],
    exists: impl Fn(&Path) -> bool,
) -> Result<Vec<String>, LaunchError> {
    if program.is_empty() {
        return Err(LaunchError::NoProgram);
    }
    // Checked with the same predicate the compat roots use, before anything is
    // assembled: a machine with no Wine cannot run a Windows program, and saying
    // so here is the difference between an answer and a sandbox error.
    if !exists(Path::new(WINE)) {
        return Err(LaunchError::NoRuntime(PathBuf::from(WINE)));
    }
    let binds = plumbing_binds(&bottle.plumbing, runtime_dir, &exists);
    let run =
        bottle_run(bottle, usr, launch_env(bottle, display), binds).map_err(LaunchError::Bottle)?;
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
    argv.push(exec.to_string());
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
            plumbing: Plumbing {
                display: Display::X11,
                gpu: false,
                fonts: true,
            },
            program: Vec::new(),
        }
    }

    fn all(_: &Path) -> bool {
        true
    }

    #[test]
    fn the_program_comes_last_after_the_separator() {
        let argv = confined_argv(
            &bottle(),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            Some(":0"),
            WINE,
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
        assert!(
            !env.contains_key("DISPLAY"),
            "a bottle that draws nothing is told nothing"
        );
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
        let err = confined_argv(
            &b,
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            None,
            WINE,
            &["notepad".into()],
            all,
        );
        assert!(matches!(err, Err(LaunchError::UnmetDrives(_))), "{err:?}");
    }

    #[test]
    fn a_machine_without_wine_refuses_before_it_assembles_anything() {
        // Everything else on this host is present; only Wine is missing, which
        // is the state of an image that ships the manager and not the runtime.
        let no_wine = |p: &Path| p != Path::new(WINE);
        let err = confined_argv(
            &bottle(),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            None,
            WINE,
            &["notepad".to_string()],
            no_wine,
        );
        assert!(matches!(err, Err(LaunchError::NoRuntime(_))), "{err:?}");
        // And it says which binary it looked for, because "install Wine" is
        // only actionable if a person knows what was not there.
        assert!(format!("{}", err.unwrap_err()).contains("/usr/bin/wine"));
    }

    #[test]
    fn a_launch_with_no_program_is_refused() {
        let err = confined_argv(
            &bottle(),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            None,
            WINE,
            &[],
            all,
        );
        assert!(matches!(err, Err(LaunchError::NoProgram)));
    }

    #[test]
    fn a_compat_root_this_host_lacks_is_not_bound() {
        let argv = confined_argv(
            &bottle(),
            Path::new("/usr"),
            Path::new("/run/user/1000"),
            None,
            WINE,
            &["notepad".into()],
            |p| p != Path::new("/lib64"),
        )
        .unwrap();
        assert!(
            !argv.iter().any(|a| a == "/lib64"),
            "bwrap fails the whole launch on a missing source"
        );
    }
}
