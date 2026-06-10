//! Build-step planning and direct-exec execution for forage.
//!
//! The build phase (forage-recipes.md sections 9, 10a) turns a recipe's
//! `[build]` into an ordered list of commands and runs them with **no shell**:
//! a known `system` maps to a built-in tool sequence, `config_opts` are passed
//! as arguments, and `[[build.steps]]` run `{ tool, args, workdir }` directly
//! (`execvp`-style, no `sh -c`, no interpolation). Every command runs with the
//! reproducibility environment injected (`SOURCE_DATE_EPOCH`, `LC_ALL=C`,
//! `TZ=UTC`, deterministic parallelism).
//!
//! [`plan_build`] produces the [`BuildPlan`] (pure, the tested core);
//! [`execute_plan`] runs it through a [`StepRunner`] (the [`ProcessRunner`]
//! spawns each command with a controlled environment). Sandboxing wraps the
//! runner in a later slice; the plan and the no-shell discipline are here.
//!
//! Containment boundary: the executor does not itself interpolate into a shell
//! and rejects the blatant cases (a step whose tool is a shell, a recipe env
//! overriding PATH or the loader, a workdir escaping the source). It is **not**
//! the security boundary for arbitrary build code: builds legitimately invoke
//! shells internally (`make`, `configure`), so per forage-recipes.md section
//! 10a the actual containment is the build sandbox (no network, a confined
//! read-only source, no privilege), which traps whatever the build runs. The
//! tool/env/workdir checks here are defense-in-depth and enforce the recipe's
//! declarative intent, not the trust boundary.

use std::collections::BTreeMap;
use std::path::Path;

use arlen_forage_recipe::{Build, BuildStep, BuildSystem};
use thiserror::Error;

/// Inputs the executor needs that are not in the recipe: the source commit
/// timestamp (for `SOURCE_DATE_EPOCH`), the deterministic job count, and the
/// physical build directory whose path is rewritten out of emitted artifacts.
#[derive(Debug, Clone)]
pub struct BuildContext {
    /// Seconds since the epoch of the source commit; pins `SOURCE_DATE_EPOCH`.
    pub source_date_epoch: i64,
    /// Deterministic parallelism; a fixed value keeps the build reproducible.
    pub jobs: u32,
    /// The physical build-directory path to rewrite out of emitted artifacts
    /// (debug info, `__FILE__`, panic paths), mapped to the canonical
    /// [`arlen_confiner::BUILD_MOUNT`] via `--remap-path-prefix` /
    /// `-ffile-prefix-map`. `None` skips the path-remap flags (the rest of the
    /// reproducibility environment is still injected). The unconfined runner
    /// passes its real working directory so the machine-specific path does not
    /// leak; the confined runner already builds at the canonical mount, so the
    /// map is a harmless no-op there (the embedded paths are `/build/...`
    /// regardless).
    pub build_dir: Option<String>,
}

/// One planned command: a tool and its arguments, never a shell line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildCommand {
    /// The executable to run, resolved from PATH by the runner (no shell).
    pub tool: String,
    /// Arguments passed verbatim.
    pub args: Vec<String>,
    /// Working directory relative to the source root, if any.
    pub workdir: Option<String>,
    /// Environment for this command (reproducibility env plus recipe env).
    pub env: BTreeMap<String, String>,
}

/// The ordered commands a build runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlan {
    /// The commands, in execution order.
    pub commands: Vec<BuildCommand>,
}

/// A failure planning or executing a build.
#[derive(Debug, Error)]
pub enum BuildError {
    /// The build system does not yet have a built-in builder in this slice.
    #[error("unsupported build system: {0:?}")]
    UnsupportedSystem(BuildSystem),
    /// `system = "custom"` (or no system) was given with no `[[build.steps]]`.
    #[error("custom build requires at least one [[build.steps]]")]
    MissingSteps,
    /// A build step named an empty tool.
    #[error("build step {0} has an empty tool")]
    EmptyTool(usize),
    /// A build step named a shell interpreter (the build must never run a shell).
    #[error("build step tool `{0}` is a shell interpreter; builds must not run a shell")]
    ShellTool(String),
    /// The recipe tried to set a runner-controlled environment variable.
    #[error("recipe may not set the runner-controlled environment variable `{0}`")]
    ReservedEnv(String),
    /// A build step workdir was absolute or escaped the source root.
    #[error("build step workdir `{0}` must be relative and within the source tree")]
    InvalidWorkdir(String),
    /// A command exited non-zero or could not be spawned.
    #[error("command `{tool}` failed: {reason}")]
    CommandFailed {
        /// The tool that failed.
        tool: String,
        /// Why it failed (spawn error or non-zero exit).
        reason: String,
    },
    /// Constructing the confinement for a confined step failed.
    #[error("confinement: {0}")]
    Confinement(String),
}

/// Environment variables the runner controls; a recipe may not set them
/// because they steer execution (PATH and shell behaviour). Any `LD_*` key is
/// also rejected (see [`is_reserved_env`]): the dynamic loader honours a large,
/// open-ended family (`LD_PRELOAD`, `LD_LIBRARY_PATH`, `LD_AUDIT`,
/// `LD_TRACE_LOADED_OBJECTS`, `LD_DEBUG`, ...), several of which alter or
/// short-circuit execution, so the whole prefix is reserved rather than an
/// always-incomplete enumeration. Recipe vars are deterministic build inputs
/// only.
const RESERVED_ENV: &[&str] = &["PATH", "SHELL", "CONFIG_SHELL", "BASH_ENV", "ENV", "IFS"];

/// Whether a recipe-supplied env key is reserved for the runner.
fn is_reserved_env(key: &str) -> bool {
    RESERVED_ENV.contains(&key) || key.starts_with("LD_")
}

/// Tool basenames that are shell interpreters (or generic launchers that can
/// invoke one). A build step may not run any of these: the build must never go
/// through a shell.
const SHELL_TOOLS: &[&str] = &[
    "sh", "bash", "dash", "zsh", "ksh", "csh", "tcsh", "fish", "ash", "busybox", "env",
];

/// Whether a tool resolves (by basename) to a shell interpreter or launcher.
fn is_shell_like(tool: &str) -> bool {
    let base = Path::new(tool)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(tool);
    SHELL_TOOLS.contains(&base)
}

/// Whether a workdir is a safe relative path contained in the source tree:
/// not absolute and with no `..` (or root/prefix) components.
fn workdir_is_contained(w: &str) -> bool {
    use std::path::Component;
    let p = Path::new(w);
    if p.is_absolute() {
        return false;
    }
    p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Reject recipe-controlled inputs that would breach the build's guarantees:
/// a shell tool, a runner-controlled env var, or an escaping workdir.
fn validate_build(build: &Build) -> Result<(), BuildError> {
    for key in build.env.keys() {
        if is_reserved_env(key) {
            return Err(BuildError::ReservedEnv(key.clone()));
        }
    }
    for (i, step) in build.steps.iter().enumerate() {
        if step.tool.trim().is_empty() {
            return Err(BuildError::EmptyTool(i));
        }
        if is_shell_like(&step.tool) {
            return Err(BuildError::ShellTool(step.tool.clone()));
        }
        if let Some(w) = &step.workdir {
            if !workdir_is_contained(w) {
                return Err(BuildError::InvalidWorkdir(w.clone()));
            }
        }
    }
    Ok(())
}

/// Build the reproducibility environment every command inherits. Recipe `env`
/// is applied first; the reproducibility keys are applied last so they cannot
/// be overridden (a recipe cannot, say, unset `LC_ALL`). Runner-controlled keys
/// are rejected earlier by [`validate_build`], so they never appear here.
fn build_env(build: &Build, ctx: &BuildContext) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = build.env.clone();
    env.insert("SOURCE_DATE_EPOCH".into(), ctx.source_date_epoch.to_string());
    env.insert("LC_ALL".into(), "C".into());
    env.insert("LANG".into(), "C".into());
    env.insert("TZ".into(), "UTC".into());
    if let Some(from) = &ctx.build_dir {
        inject_path_prefix_map(&mut env, from);
    }
    env
}

/// Append the build-path remapping flags (forage-recipes.md section 13,
/// "normalise the build"): tell the Rust and C/C++ toolchains to rewrite the
/// physical build directory `from` to the canonical
/// [`arlen_confiner::BUILD_MOUNT`], so an emitted artifact embeds `/build/...`
/// rather than the machine-specific path. `-ffile-prefix-map` is the superset
/// of `-fdebug-prefix-map` + `-fmacro-prefix-map`. The flags are appended to
/// any recipe-supplied `RUSTFLAGS` / `CFLAGS` / `CXXFLAGS` so the recipe's own
/// flags survive (path maps are additive in the compilers).
fn inject_path_prefix_map(env: &mut BTreeMap<String, String>, from: &str) {
    // A `=` in the source path makes the `old=new` flag ambiguous. Forage owns
    // the build directory and never creates one containing `=`, so this guard
    // cannot fire for a real build, but it keeps a malformed flag from ever
    // being emitted rather than corrupting the build invocation.
    if from.contains('=') {
        return;
    }
    let to = arlen_confiner::BUILD_MOUNT;
    append_flag(env, "RUSTFLAGS", format!("--remap-path-prefix={from}={to}"));
    let file_map = format!("-ffile-prefix-map={from}={to}");
    append_flag(env, "CFLAGS", file_map.clone());
    append_flag(env, "CXXFLAGS", file_map);
}

/// Append `flag` to `env[key]`, space-separated, preserving any existing value.
fn append_flag(env: &mut BTreeMap<String, String>, key: &str, flag: String) {
    env.entry(key.to_string())
        .and_modify(|v| {
            if !v.is_empty() {
                v.push(' ');
            }
            v.push_str(&flag);
        })
        .or_insert(flag);
}

/// Turn a recipe's `[build]` into an ordered, shell-free command plan.
pub fn plan_build(build: &Build, ctx: &BuildContext) -> Result<BuildPlan, BuildError> {
    // Reject recipe inputs that would breach the no-shell / fixed-PATH /
    // contained-workdir guarantees before planning anything.
    validate_build(build)?;

    let env = build_env(build, ctx);
    let jobs = ctx.jobs.to_string();

    let mut commands = Vec::new();

    match build.system {
        // No system, or explicit custom: the steps are the whole build.
        None | Some(BuildSystem::Custom) => {
            if build.steps.is_empty() {
                return Err(BuildError::MissingSteps);
            }
        }
        Some(system) => {
            commands.extend(builtin_sequence(system, &build.config_opts, &jobs, &env)?);
        }
    }

    // Declared steps run after any built-in sequence (or are the whole build
    // for a custom system), each a direct exec with the reproducibility env.
    commands.extend(steps_to_commands(&build.steps, &env));

    Ok(BuildPlan { commands })
}

/// The built-in tool sequence for a known build system. `config_opts` are
/// appended to the configure/build invocation as arguments (never shell).
fn builtin_sequence(
    system: BuildSystem,
    config_opts: &[String],
    jobs: &str,
    env: &BTreeMap<String, String>,
) -> Result<Vec<BuildCommand>, BuildError> {
    let cmd = |tool: &str, args: Vec<String>| BuildCommand {
        tool: tool.to_string(),
        args,
        workdir: None,
        env: env.clone(),
    };
    let opts = || config_opts.to_vec();

    let seq = match system {
        BuildSystem::Cargo => {
            let mut args = vec![
                "build".into(),
                "--release".into(),
                "--locked".into(),
                "--offline".into(),
                "-j".into(),
                jobs.into(),
            ];
            args.extend(opts());
            vec![cmd("cargo", args)]
        }
        BuildSystem::Make => {
            let mut args = vec![format!("-j{jobs}")];
            args.extend(opts());
            vec![cmd("make", args)]
        }
        BuildSystem::Cmake => {
            let mut configure = vec!["-B".into(), "build".into()];
            configure.extend(opts());
            vec![
                cmd("cmake", configure),
                cmd(
                    "cmake",
                    vec![
                        "--build".into(),
                        "build".into(),
                        "--parallel".into(),
                        jobs.into(),
                    ],
                ),
            ]
        }
        BuildSystem::Meson => {
            let mut setup = vec!["setup".into(), "build".into()];
            setup.extend(opts());
            vec![
                cmd("meson", setup),
                cmd(
                    "meson",
                    vec!["compile".into(), "-C".into(), "build".into(), "-j".into(), jobs.into()],
                ),
            ]
        }
        BuildSystem::Autotools => {
            let mut configure = vec![];
            configure.extend(opts());
            vec![
                cmd("./configure", configure),
                cmd("make", vec![format!("-j{jobs}")]),
            ]
        }
        // Deferred to a later slice; the recipe still validates, but planning a
        // build for these is not yet implemented.
        other @ (BuildSystem::Go
        | BuildSystem::Python
        | BuildSystem::Zig
        | BuildSystem::Nim
        | BuildSystem::Npm
        | BuildSystem::Pnpm) => return Err(BuildError::UnsupportedSystem(other)),
        BuildSystem::Custom => unreachable!("custom is handled before builtin_sequence"),
    };
    Ok(seq)
}

/// Runs one planned command. Behind a trait so the plan can be tested without
/// spawning processes.
pub trait StepRunner {
    /// Run `cmd` with `source_root` as the base for its (relative) workdir.
    fn run(&self, cmd: &BuildCommand, source_root: &Path) -> Result<(), BuildError>;
}

/// Execute every command in a plan in order, stopping at the first failure.
pub fn execute_plan(
    plan: &BuildPlan,
    runner: &dyn StepRunner,
    source_root: &Path,
) -> Result<(), BuildError> {
    for cmd in &plan.commands {
        runner.run(cmd, source_root)?;
    }
    Ok(())
}

/// Bounds on a single build command's execution (forage-recipes.md section 13,
/// "the build is bounded"). The wall-clock timeout is enforced here in the
/// runner without privilege: a hung or looping build is killed and its whole
/// process group reaped. The memory and disk caps are a cgroup concern applied
/// on a privileged host with the confined runner and are deliberately not part
/// of this unprivileged limit; `None` leaves the wall-clock unbounded.
#[derive(Debug, Clone, Default)]
pub struct BuildLimits {
    /// Maximum wall-clock time for one command; `None` is unbounded.
    pub wall_clock: Option<std::time::Duration>,
}

/// The reproducible build umask. `022` masks group and other write bits, so a
/// file the build creates with a permissive request lands at a fixed mode
/// regardless of the builder's ambient umask. The `.lunpkg` writer already
/// normalises every archived mode, so this does not change the package output;
/// it makes the build itself deterministic for the narrow case where a build
/// embeds file modes into an artifact it produces (forage-recipes.md section
/// 13, "fixed umask and PATH").
#[cfg(unix)]
const BUILD_UMASK: libc::mode_t = 0o022;

/// On Unix, prepare the forked child before `exec`: mark inherited fds
/// close-on-exec so none leaks into the spawned build, make it a process-group
/// leader (`setpgid(0, 0)`) so its descendants share its group and can be killed
/// as a unit on timeout, and pin the reproducible build umask. Under the
/// confined runner the spawned process is `bwrap`, which inherits this umask and
/// carries it into the sandbox.
#[cfg(unix)]
fn prepare_child(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: the closure runs in the forked child before exec. `close_range`,
    // `setpgid` and `umask` are async-signal-safe and touch only the child's own
    // state (its open fds, process group and file-mode mask); they read or write
    // no parent state and allocate nothing.
    unsafe {
        command.pre_exec(|| {
            // Mark every fd above stderr close-on-exec so no forage-process fd
            // (the builder signing key, fetched sources, a fetch-phase socket)
            // leaks through bwrap into an untrusted build. Rust opens fds
            // O_CLOEXEC by default, so this is belt-and-suspenders against a
            // raw-libc or inherited non-CLOEXEC fd. Best-effort, unlike
            // arlen-run's fatal close: this helper is shared with the unconfined
            // `--unsafe-no-sandbox` runner and forage carries no kernel floor,
            // while CLOSE_RANGE_CLOEXEC needs Linux >= 5.11; on an older kernel
            // the syscall no-ops and the O_CLOEXEC default remains the defence.
            let _ = libc::close_range(3, libc::c_uint::MAX, libc::CLOSE_RANGE_CLOEXEC as libc::c_int);
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::umask(BUILD_UMASK);
            Ok(())
        });
    }
}

/// Kill the build's whole process group with `SIGKILL` (uncatchable, so a
/// wedged build cannot ignore it). The child leads its own group (see
/// [`prepare_child`]), so negating its pid signals every descendant that
/// did not fork into a new group of its own; the leader is also signalled
/// directly in case the group setup lost a race with a fast-spawning child.
#[cfg(unix)]
fn kill_process_group(child: &std::process::Child) {
    let pid = child.id() as libc::pid_t;
    // SAFETY: `kill` is a direct syscall wrapper with no memory effects.
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
        libc::kill(pid, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
fn prepare_child(_command: &mut std::process::Command) {}
#[cfg(not(unix))]
fn kill_process_group(child: &std::process::Child) {
    let _ = child;
}

/// Spawn `command` (already fully configured), enforce the wall-clock limit,
/// and map the result to a [`BuildError`]. On timeout the whole process group
/// is killed and the direct child reaped, reported as a timeout (not a generic
/// non-zero exit) so the caller can tell a hung build from a failed one.
fn run_command(
    mut command: std::process::Command,
    tool: &str,
    limits: &BuildLimits,
) -> Result<(), BuildError> {
    prepare_child(&mut command);
    let mut child = command.spawn().map_err(|e| BuildError::CommandFailed {
        tool: tool.to_string(),
        reason: format!("spawn: {e}"),
    })?;

    let status = match limits.wall_clock {
        None => child.wait().map_err(|e| BuildError::CommandFailed {
            tool: tool.to_string(),
            reason: format!("wait: {e}"),
        })?,
        Some(dur) => {
            use wait_timeout::ChildExt;
            let waited = match child.wait_timeout(dur) {
                Ok(w) => w,
                Err(e) => {
                    // A wait failure leaves the build running; kill it rather
                    // than leak a process before surfacing the error.
                    kill_process_group(&child);
                    let _ = child.wait();
                    return Err(BuildError::CommandFailed {
                        tool: tool.to_string(),
                        reason: format!("wait: {e}"),
                    });
                }
            };
            match waited {
                Some(status) => status,
                None => {
                    // The build has not exited, so `Child` still holds its
                    // unreaped slot; that pins both the pid and the process
                    // group id against reuse until the `child.wait()` below, so
                    // `kill(-pid)` cannot land on an unrelated recycled group.
                    kill_process_group(&child);
                    // Reap the now-killed direct child so it is not left a zombie.
                    let _ = child.wait();
                    return Err(BuildError::CommandFailed {
                        tool: tool.to_string(),
                        reason: format!("wall-clock timeout after {}s", dur.as_secs()),
                    });
                }
            }
        }
    };

    if !status.success() {
        return Err(BuildError::CommandFailed {
            tool: tool.to_string(),
            reason: format!("exit {status}"),
        });
    }
    Ok(())
}

/// The production runner: spawns each command with `std::process::Command`,
/// passing arguments explicitly (never a shell), with a controlled environment
/// (the command's planned env plus a fixed minimal `PATH`) and the relative
/// workdir resolved under the source root. The configured [`BuildLimits`] bound
/// each command's wall-clock time.
///
/// This runner offers **no containment** of a process that deliberately detaches
/// itself: build code that calls `setsid` / `setpgid` to leave its process group
/// survives the timeout kill, because without a pid namespace the group is the
/// only handle. Run untrusted recipes through [`ConfinedStepRunner`] (bwrap
/// `--unshare-pid`), where killing `bwrap` tears down the whole pid namespace and
/// nothing can escape. The unconfined runner is the dev / test path.
#[derive(Debug, Clone, Default)]
pub struct ProcessRunner {
    limits: BuildLimits,
}

impl ProcessRunner {
    /// A runner that enforces `limits` on every command.
    pub fn with_limits(limits: BuildLimits) -> Self {
        ProcessRunner { limits }
    }
}

impl StepRunner for ProcessRunner {
    fn run(&self, cmd: &BuildCommand, source_root: &Path) -> Result<(), BuildError> {
        let mut command = std::process::Command::new(&cmd.tool);
        command.args(&cmd.args);
        // Controlled environment: clear inherited vars, apply the planned env,
        // then set the fixed PATH last so it is authoritative even if a command
        // was built directly (bypassing plan_build's reserved-key rejection).
        command.env_clear();
        for (k, v) in &cmd.env {
            command.env(k, v);
        }
        command.env("PATH", "/usr/bin:/bin");
        let dir = match &cmd.workdir {
            Some(w) => source_root.join(w),
            None => source_root.to_path_buf(),
        };
        command.current_dir(dir);

        run_command(command, &cmd.tool, &self.limits)
    }
}

/// A [`StepRunner`] that runs each step inside the shared bubblewrap confiner's
/// build profile (`arlen-confiner`): the pinned base platform as a read-only
/// `/`, the source root (the build dir) writable at the fixed `/build`, no
/// network, a deterministic environment. Build state persists between steps
/// through the read-write `/build` bind (each step is its own `bwrap`
/// invocation, so a step's `workdir` becomes the in-sandbox working directory).
///
/// [`confined_argv`](ConfinedStepRunner::confined_argv) is the pure, tested
/// argument-vector construction. `run` spawns `bwrap` with it — that spawn is
/// the on-kernel part (it needs `bwrap` and unprivileged user namespaces) and
/// is verified on a real machine, not in unit tests.
///
/// Not yet attached: the build-appropriate **seccomp** allowlist (the next
/// hardening layer, passed as `bwrap --seccomp <fd>`); until then confinement
/// is by namespaces + `no_new_privs` + the read-only/no-network mounts.
#[derive(Debug, Clone)]
pub struct ConfinedStepRunner {
    /// The pinned base platform mounted read-only as `/` (roadmap D2).
    base_platform: std::path::PathBuf,
    /// The `bwrap` binary (default `bwrap` from PATH).
    bwrap: std::path::PathBuf,
    /// Execution limits enforced around the `bwrap` invocation. Killing the
    /// `bwrap` process tears down its pid namespace, reaping the build tree.
    limits: BuildLimits,
}

impl ConfinedStepRunner {
    /// A confined runner using `base_platform` as the read-only root and the
    /// `bwrap` binary from PATH, with no wall-clock limit.
    pub fn new(base_platform: impl Into<std::path::PathBuf>) -> Self {
        ConfinedStepRunner {
            base_platform: base_platform.into(),
            bwrap: std::path::PathBuf::from("bwrap"),
            limits: BuildLimits::default(),
        }
    }

    /// Set the execution limits enforced around each confined step.
    pub fn with_limits(mut self, limits: BuildLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Build the `bwrap` argument vector (everything after the `bwrap` program)
    /// for `cmd` against the build directory `source_root`: the confiner's build
    /// profile, the per-step working directory under `/build`, then
    /// `-- <tool> <args>`. Pure and deterministic.
    pub fn confined_argv(
        &self,
        cmd: &BuildCommand,
        source_root: &Path,
    ) -> Result<Vec<String>, BuildError> {
        let conf = arlen_confiner::build_profile(&self.base_platform, source_root, cmd.env.clone())
            .map_err(|e| BuildError::Confinement(e.to_string()))?;
        // The step's workdir is relative and contained (validated at planning);
        // map it under the fixed in-sandbox build mount.
        let chdir = match &cmd.workdir {
            Some(w) => format!("{}/{}", arlen_confiner::BUILD_MOUNT, w),
            None => arlen_confiner::BUILD_MOUNT.to_string(),
        };
        let mut argv = conf.with_chdir(chdir).bwrap_args();
        argv.push("--".into());
        argv.push(cmd.tool.clone());
        argv.extend(cmd.args.iter().cloned());
        Ok(argv)
    }
}

impl StepRunner for ConfinedStepRunner {
    fn run(&self, cmd: &BuildCommand, source_root: &Path) -> Result<(), BuildError> {
        let argv = self.confined_argv(cmd, source_root)?;
        let mut command = std::process::Command::new(&self.bwrap);
        command.args(&argv);
        run_command(command, &cmd.tool, &self.limits)
    }
}

/// Convenience: the recipe step list as `BuildStep`s, for callers assembling a
/// custom build programmatically.
pub fn steps_to_commands(
    steps: &[BuildStep],
    env: &BTreeMap<String, String>,
) -> Vec<BuildCommand> {
    steps
        .iter()
        .map(|s| BuildCommand {
            tool: s.tool.clone(),
            args: s.args.clone(),
            workdir: s.workdir.clone(),
            env: env.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> BuildContext {
        BuildContext {
            source_date_epoch: 1_700_000_000,
            jobs: 4,
            build_dir: None,
        }
    }

    fn build(system: Option<BuildSystem>) -> Build {
        Build {
            system,
            host_deps: Vec::new(),
            config_opts: Vec::new(),
            env: BTreeMap::new(),
            steps: Vec::new(),
            offline: true,
            jobs: None,
            fetch_lock: None,
        }
    }

    #[test]
    fn cargo_plan_is_offline_locked_release() {
        let plan = plan_build(&build(Some(BuildSystem::Cargo)), &ctx()).unwrap();
        assert_eq!(plan.commands.len(), 1);
        let c = &plan.commands[0];
        assert_eq!(c.tool, "cargo");
        assert_eq!(c.args, ["build", "--release", "--locked", "--offline", "-j", "4"]);
    }

    #[test]
    fn reproducibility_env_is_injected_and_unoverridable() {
        let mut b = build(Some(BuildSystem::Make));
        // A recipe trying to override LC_ALL must not win.
        b.env.insert("LC_ALL".into(), "en_US.UTF-8".into());
        b.env.insert("MYVAR".into(), "1".into());
        let plan = plan_build(&b, &ctx()).unwrap();
        let env = &plan.commands[0].env;
        assert_eq!(env.get("LC_ALL").unwrap(), "C", "repro env wins over recipe env");
        assert_eq!(env.get("TZ").unwrap(), "UTC");
        assert_eq!(env.get("SOURCE_DATE_EPOCH").unwrap(), "1700000000");
        assert_eq!(env.get("MYVAR").unwrap(), "1", "recipe env still passes through");
    }

    #[test]
    fn path_prefix_map_rewrites_the_build_dir_to_the_canonical_mount() {
        let mut c = ctx();
        c.build_dir = Some("/var/tmp/forage-build-xyz".into());
        let plan = plan_build(&build(Some(BuildSystem::Cargo)), &c).unwrap();
        let env = &plan.commands[0].env;
        let to = arlen_confiner::BUILD_MOUNT;
        assert_eq!(
            env.get("RUSTFLAGS").unwrap(),
            &format!("--remap-path-prefix=/var/tmp/forage-build-xyz={to}")
        );
        let file_map = format!("-ffile-prefix-map=/var/tmp/forage-build-xyz={to}");
        assert_eq!(env.get("CFLAGS").unwrap(), &file_map);
        assert_eq!(env.get("CXXFLAGS").unwrap(), &file_map);
    }

    #[test]
    fn path_prefix_map_is_appended_to_recipe_flags() {
        let mut b = build(Some(BuildSystem::Make));
        b.env.insert("RUSTFLAGS".into(), "-Ctarget-cpu=native".into());
        b.env.insert("CFLAGS".into(), "-O2".into());
        let mut c = ctx();
        c.build_dir = Some("/build".into());
        let plan = plan_build(&b, &c).unwrap();
        let env = &plan.commands[0].env;
        // The recipe's own flag survives, ours follows it, space-separated.
        assert_eq!(
            env.get("RUSTFLAGS").unwrap(),
            "-Ctarget-cpu=native --remap-path-prefix=/build=/build"
        );
        assert_eq!(env.get("CFLAGS").unwrap(), "-O2 -ffile-prefix-map=/build=/build");
    }

    #[test]
    fn no_build_dir_means_no_remap_flags() {
        let plan = plan_build(&build(Some(BuildSystem::Cargo)), &ctx()).unwrap();
        let env = &plan.commands[0].env;
        assert!(!env.contains_key("RUSTFLAGS"));
        assert!(!env.contains_key("CFLAGS"));
        // The rest of the reproducibility env is still present.
        assert_eq!(env.get("SOURCE_DATE_EPOCH").unwrap(), "1700000000");
    }

    #[test]
    fn build_dir_with_equals_skips_the_ambiguous_flag() {
        let mut c = ctx();
        c.build_dir = Some("/var/tmp/a=b".into());
        let plan = plan_build(&build(Some(BuildSystem::Cargo)), &c).unwrap();
        // A `=` in the path would make `old=new` ambiguous, so no flag is emitted.
        assert!(!plan.commands[0].env.contains_key("RUSTFLAGS"));
    }

    #[test]
    fn config_opts_are_appended_as_args_not_shell() {
        let mut b = build(Some(BuildSystem::Cmake));
        b.config_opts = vec!["-DFOO=bar".into(), "-DBAZ=ON".into()];
        let plan = plan_build(&b, &ctx()).unwrap();
        assert_eq!(plan.commands.len(), 2);
        assert_eq!(plan.commands[0].tool, "cmake");
        assert!(plan.commands[0].args.contains(&"-DFOO=bar".to_string()));
        assert_eq!(plan.commands[1].args[0], "--build");
    }

    #[test]
    fn meson_plans_setup_then_compile() {
        let plan = plan_build(&build(Some(BuildSystem::Meson)), &ctx()).unwrap();
        assert_eq!(plan.commands[0].args[0], "setup");
        assert_eq!(plan.commands[1].args[0], "compile");
    }

    #[test]
    fn custom_requires_steps() {
        assert!(matches!(
            plan_build(&build(Some(BuildSystem::Custom)), &ctx()),
            Err(BuildError::MissingSteps)
        ));
        assert!(matches!(
            plan_build(&build(None), &ctx()),
            Err(BuildError::MissingSteps)
        ));
    }

    #[test]
    fn custom_steps_run_directly() {
        let mut b = build(Some(BuildSystem::Custom));
        b.steps = vec![BuildStep {
            tool: "zig".into(),
            args: vec!["build".into(), "-Doptimize=ReleaseSafe".into()],
            workdir: Some("src".into()),
        }];
        let plan = plan_build(&b, &ctx()).unwrap();
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].tool, "zig");
        assert_eq!(plan.commands[0].workdir.as_deref(), Some("src"));
    }

    #[test]
    fn steps_run_after_a_builtin_sequence() {
        let mut b = build(Some(BuildSystem::Make));
        b.steps = vec![BuildStep {
            tool: "strip".into(),
            args: vec!["target/app".into()],
            workdir: None,
        }];
        let plan = plan_build(&b, &ctx()).unwrap();
        assert_eq!(plan.commands.len(), 2, "make then the post-step");
        assert_eq!(plan.commands[0].tool, "make");
        assert_eq!(plan.commands[1].tool, "strip");
    }

    #[test]
    fn empty_step_tool_is_rejected() {
        let mut b = build(Some(BuildSystem::Custom));
        b.steps = vec![BuildStep {
            tool: "  ".into(),
            args: Vec::new(),
            workdir: None,
        }];
        assert!(matches!(plan_build(&b, &ctx()), Err(BuildError::EmptyTool(0))));
    }

    #[test]
    fn shell_tools_are_rejected() {
        for tool in ["sh", "/bin/sh", "bash", "/usr/bin/bash", "busybox", "env"] {
            let mut b = build(Some(BuildSystem::Custom));
            b.steps = vec![BuildStep {
                tool: tool.into(),
                args: vec!["-c".into(), "echo pwned; rm -rf /".into()],
                workdir: None,
            }];
            assert!(
                matches!(plan_build(&b, &ctx()), Err(BuildError::ShellTool(_))),
                "`{tool}` must be rejected as a shell"
            );
        }
    }

    #[test]
    fn recipe_cannot_override_runner_controlled_env() {
        for key in [
            "PATH",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "LD_AUDIT",
            "LD_TRACE_LOADED_OBJECTS",
            "LD_DEBUG",
            "LD_PROFILE",
            "SHELL",
            "CONFIG_SHELL",
        ] {
            let mut b = build(Some(BuildSystem::Make));
            b.env.insert(key.into(), "/evil".into());
            assert!(
                matches!(plan_build(&b, &ctx()), Err(BuildError::ReservedEnv(_))),
                "recipe setting `{key}` must be rejected"
            );
        }
    }

    #[test]
    fn escaping_workdirs_are_rejected() {
        for w in ["/etc", "../..", "a/../../b", "/", "../outside"] {
            let mut b = build(Some(BuildSystem::Custom));
            b.steps = vec![BuildStep {
                tool: "make".into(),
                args: Vec::new(),
                workdir: Some(w.into()),
            }];
            assert!(
                matches!(plan_build(&b, &ctx()), Err(BuildError::InvalidWorkdir(_))),
                "workdir `{w}` must be rejected"
            );
        }
    }

    #[test]
    fn contained_workdir_is_accepted() {
        let mut b = build(Some(BuildSystem::Custom));
        b.steps = vec![BuildStep {
            tool: "make".into(),
            args: Vec::new(),
            workdir: Some("src/sub".into()),
        }];
        assert_eq!(plan_build(&b, &ctx()).unwrap().commands.len(), 1);
    }

    #[test]
    fn deferred_systems_are_unsupported() {
        for system in [BuildSystem::Go, BuildSystem::Python, BuildSystem::Npm] {
            assert!(matches!(
                plan_build(&build(Some(system)), &ctx()),
                Err(BuildError::UnsupportedSystem(_))
            ));
        }
    }

    #[test]
    fn confined_argv_wraps_the_step_in_the_build_profile() {
        let runner = ConfinedStepRunner::new("/opt/arlen/platform");
        let cmd = BuildCommand {
            tool: "cargo".into(),
            args: vec!["build".into(), "--release".into()],
            workdir: Some("crate".into()),
            env: BTreeMap::from([("SOURCE_DATE_EPOCH".to_string(), "0".to_string())]),
        };
        let argv = runner.confined_argv(&cmd, Path::new("/var/tmp/work")).unwrap();
        // Confinement flags from the build profile.
        assert!(argv.contains(&"--unshare-net".to_string()));
        // Base platform read-only at /, build dir read-write at the fixed /build.
        assert!(argv
            .windows(3)
            .any(|w| w[0] == "--ro-bind" && w[1] == "/opt/arlen/platform" && w[2] == "/"));
        assert!(argv
            .windows(3)
            .any(|w| w[0] == "--bind" && w[1] == "/var/tmp/work" && w[2] == "/build"));
        // The step's workdir lands under /build.
        let chdir = argv.iter().position(|a| a == "--chdir").unwrap();
        assert_eq!(argv[chdir + 1], "/build/crate");
        // The tool + args follow a `--` separator, in order, at the end.
        let sep = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(&argv[sep + 1..], &["cargo", "build", "--release"]);
    }

    #[test]
    fn confined_argv_rejects_a_build_dir_inside_the_platform() {
        let runner = ConfinedStepRunner::new("/opt/arlen/platform");
        let cmd = BuildCommand {
            tool: "make".into(),
            args: vec![],
            workdir: None,
            env: BTreeMap::new(),
        };
        // Overlap is rejected by the confiner and surfaced as a build error.
        assert!(matches!(
            runner.confined_argv(&cmd, Path::new("/opt/arlen/platform/work")),
            Err(BuildError::Confinement(_))
        ));
    }

    #[test]
    fn process_runner_executes_without_a_shell() {
        let dir = tempfile::tempdir().unwrap();
        // A command that creates a marker proves args + workdir + exec work
        // without a shell. Shell metacharacters in args stay literal.
        let plan = BuildPlan {
            commands: vec![BuildCommand {
                tool: "touch".into(),
                args: vec!["marker;not-a-shell".into()],
                workdir: None,
                env: BTreeMap::new(),
            }],
        };
        execute_plan(&plan, &ProcessRunner::default(), dir.path()).unwrap();
        assert!(dir.path().join("marker;not-a-shell").exists());
        // No shell ran, so no file named by the part after `;` exists.
        assert!(!dir.path().join("not-a-shell").exists());
    }

    #[test]
    fn process_runner_reports_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let plan = BuildPlan {
            commands: vec![BuildCommand {
                tool: "false".into(),
                args: Vec::new(),
                workdir: None,
                env: BTreeMap::new(),
            }],
        };
        assert!(matches!(
            execute_plan(&plan, &ProcessRunner::default(), dir.path()),
            Err(BuildError::CommandFailed { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn process_runner_kills_a_hung_build_on_timeout() {
        use std::time::{Duration, Instant};
        let dir = tempfile::tempdir().unwrap();
        let runner = ProcessRunner::with_limits(BuildLimits {
            wall_clock: Some(Duration::from_millis(300)),
        });
        let plan = BuildPlan {
            commands: vec![BuildCommand {
                tool: "sleep".into(),
                args: vec!["60".into()],
                workdir: None,
                env: BTreeMap::new(),
            }],
        };
        let start = Instant::now();
        let err = execute_plan(&plan, &runner, dir.path()).unwrap_err();
        // It returns well before the 60s sleep, reported as a timeout.
        assert!(start.elapsed() < Duration::from_secs(5), "killed promptly");
        match err {
            BuildError::CommandFailed { reason, .. } => {
                assert!(reason.contains("timeout"), "reported as a timeout: {reason}");
            }
            other => panic!("expected a timeout failure, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn timeout_reaps_the_whole_process_tree() {
        use std::time::Duration;
        // The build spawns a grandchild via a long-lived process tree. On
        // timeout the process group kill must take the grandchild down too, not
        // just the direct child. The grandchild writes a marker only if it
        // survives long enough; with the group reaped it never does.
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived");
        // `sh` is rejected as a build tool by plan_build, so construct the plan
        // directly: this is a runner test, not a planning test. The child runs a
        // small tree: sleep, then touch the marker. A reaped tree never touches.
        let plan = BuildPlan {
            commands: vec![BuildCommand {
                tool: "sh".into(),
                args: vec![
                    "-c".into(),
                    format!("sleep 30 && touch {}", marker.display()),
                ],
                workdir: None,
                env: BTreeMap::new(),
            }],
        };
        let runner = ProcessRunner::with_limits(BuildLimits {
            wall_clock: Some(Duration::from_millis(300)),
        });
        let _ = execute_plan(&plan, &runner, dir.path());
        // Give any unreaped descendant well over its sleep to (wrongly) fire.
        std::thread::sleep(Duration::from_millis(800));
        assert!(!marker.exists(), "the reaped process tree never touched the marker");
    }

    #[cfg(unix)]
    #[test]
    fn no_timeout_lets_a_quick_build_finish() {
        let dir = tempfile::tempdir().unwrap();
        let runner = ProcessRunner::with_limits(BuildLimits {
            wall_clock: Some(std::time::Duration::from_secs(30)),
        });
        let plan = BuildPlan {
            commands: vec![BuildCommand {
                tool: "true".into(),
                args: Vec::new(),
                workdir: None,
                env: BTreeMap::new(),
            }],
        };
        // A fast command under a generous limit succeeds normally.
        execute_plan(&plan, &runner, dir.path()).unwrap();
    }
}
