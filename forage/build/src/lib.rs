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
/// timestamp (for `SOURCE_DATE_EPOCH`) and the deterministic job count.
#[derive(Debug, Clone)]
pub struct BuildContext {
    /// Seconds since the epoch of the source commit; pins `SOURCE_DATE_EPOCH`.
    pub source_date_epoch: i64,
    /// Deterministic parallelism; a fixed value keeps the build reproducible.
    pub jobs: u32,
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
    env
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

/// The production runner: spawns each command with `std::process::Command`,
/// passing arguments explicitly (never a shell), with a controlled environment
/// (the command's planned env plus a fixed minimal `PATH`) and the relative
/// workdir resolved under the source root.
#[derive(Debug, Default)]
pub struct ProcessRunner;

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

        let status = command.status().map_err(|e| BuildError::CommandFailed {
            tool: cmd.tool.clone(),
            reason: format!("spawn: {e}"),
        })?;
        if !status.success() {
            return Err(BuildError::CommandFailed {
                tool: cmd.tool.clone(),
                reason: format!("exit {status}"),
            });
        }
        Ok(())
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
}

impl ConfinedStepRunner {
    /// A confined runner using `base_platform` as the read-only root and the
    /// `bwrap` binary from PATH.
    pub fn new(base_platform: impl Into<std::path::PathBuf>) -> Self {
        ConfinedStepRunner {
            base_platform: base_platform.into(),
            bwrap: std::path::PathBuf::from("bwrap"),
        }
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
        let status = std::process::Command::new(&self.bwrap)
            .args(&argv)
            .status()
            .map_err(|e| BuildError::CommandFailed {
                tool: cmd.tool.clone(),
                reason: format!("spawn bwrap: {e}"),
            })?;
        if !status.success() {
            return Err(BuildError::CommandFailed {
                tool: cmd.tool.clone(),
                reason: format!("confined exit {status}"),
            });
        }
        Ok(())
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
        execute_plan(&plan, &ProcessRunner, dir.path()).unwrap();
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
            execute_plan(&plan, &ProcessRunner, dir.path()),
            Err(BuildError::CommandFailed { .. })
        ));
    }
}
