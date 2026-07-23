//! `arlen-run` - the confined app launcher.
//!
//! A fork-exec binary (not a daemon) that the shell's `launch_app` execs with an
//! app identity and a program to run. `arlen-run` loads the app's permission
//! profile, applies Landlock, a per-command cgroup and the egress seam, and
//! spawns the app under bwrap, becoming its long-lived confined parent. It
//! replaces the unconfined `sh -c` launch path.
//!
//! Fail-closed is the whole point: any setup failure - a missing/unparsable
//! profile, a confinement-setup error, an egress-filter failure - means the app
//! NEVER starts. There is no "run with reduced confinement" path; a missing
//! profile is a deny, not a default-open.
//!
//! The launcher spawns the app under bwrap with the namespace + mount
//! confinement (the pruned mount view, `no_new_privs`, `--clearenv`), applies
//! Landlock over the writable set, places the launch in a per-command cgroup
//! (reaping), and installs the egress seam. The app seccomp filter and the real
//! egress enforcer are the remaining confinement layers. A profile that asks for
//! a filtered host set refuses to launch until the real egress filter exists,
//! rather than running with unfiltered network.

use std::path::PathBuf;
use std::process::ExitCode;

#[cfg(target_os = "linux")]
mod cgroup;
mod egress;
#[cfg(target_os = "linux")]
mod landlock_apply;
// The in-sandbox Landlock wrapper mode: bwrap execs arlen-run in this mode as the
// app's stand-in so Landlock confines the app, not bwrap's own setup.
#[cfg(target_os = "linux")]
mod landlock_exec;
mod netns;
mod profile;
// The app seccomp filter (GAP-6): the deny-by-default allowlist, compiled to
// cBPF and handed to bwrap via --seccomp in `spawn`.
#[cfg(target_os = "linux")]
mod seccomp;
mod spawn;

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

    // The in-sandbox Landlock wrapper mode. bwrap runs `arlen-run --landlock-exec
    // <writable>... -- <program>...` as the app's stand-in: apply the fence, then
    // exec the app, so Landlock confines the app rather than bwrap's own setup.
    // Only returns on failure (a successful exec replaces this process).
    #[cfg(target_os = "linux")]
    if argv.first().map(String::as_str) == Some("--landlock-exec") {
        return match landlock_exec::landlock_exec(&argv[1..]) {
            Ok(never) => match never {},
            Err(landlock_exec::LandlockExecError::NoSeparator)
            | Err(landlock_exec::LandlockExecError::NoProgram) => {
                eprintln!("arlen-run --landlock-exec: malformed arguments");
                ExitCode::from(exit::BAD_ARGS)
            }
            Err(landlock_exec::LandlockExecError::Landlock(e)) => {
                eprintln!("arlen-run --landlock-exec: landlock: {e}");
                ExitCode::from(exit::CONFINE_SETUP)
            }
            Err(landlock_exec::LandlockExecError::Exec(e)) => {
                eprintln!("arlen-run --landlock-exec: exec: {e}");
                ExitCode::from(exit::SPAWN)
            }
        };
    }

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
    // profile, then build the confinement and spawn the app under bwrap. Landlock,
    // the per-command cgroup and the egress seam are applied in the spawn; the
    // seccomp filter and the real egress enforcer are the remaining layers.
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

    // Surface any host-filesystem custom grant that was refused: the drop
    // happened in `confinement_inputs` (portal-only-FS, Tier-A #3), so the
    // operator otherwise sees no reason the declared path did not take effect.
    for custom in &profile.filesystem.custom {
        if profile::is_host_escape(custom, &home) {
            eprintln!(
                "arlen-run: {}: refusing host-filesystem grant {} (not bound)",
                args.app_id,
                custom.display()
            );
        }
    }

    // A profile that declared specific hosts has its egress installed through
    // the enforcer seam. The stand-in refuses a non-empty host set until the
    // real netns proxy is wired (fail-closed: never run a host-restricted app
    // with unfiltered network); the real `EgressEnforcer` slots in here. The
    // guard is held for the whole launch and tears the restriction down on drop.
    // `None` (no network) and `Unrestricted` (no filter by design) never reach
    // the enforcer.
    use egress::EgressEnforcer;
    // A FilteredHosts profile runs in a route-absent netns behind the forwarding
    // proxy the enforcer binds. Capture the flag before `inputs.network` moves
    // into the confinement, and hold the guard for the whole launch - its Drop
    // stops the proxy. None/Unrestricted never reach the enforcer.
    let filtered = matches!(&inputs.network, arlen_confiner::NetworkPolicy::FilteredHosts(_));
    let egress_guard = if let arlen_confiner::NetworkPolicy::FilteredHosts(hosts) = &inputs.network {
        match egress::ProxyEgressEnforcer.install(hosts) {
            Ok(guard) => Some(guard),
            Err(e) => {
                eprintln!("arlen-run: {}: {e}", args.app_id);
                return ExitCode::from(exit::EGRESS);
            }
        }
    } else {
        None
    };
    let proxy_port = egress_guard.as_ref().and_then(|g| g.proxy_port());

    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let wayland_display = std::env::var("WAYLAND_DISPLAY").ok();
    let mut plumbing = match &runtime_dir {
        Some(rt) => spawn::plumbing_binds(rt, wayland_display.as_deref(), |p| p.exists()),
        None => Vec::new(),
    };
    // Bind arlen-run itself read-only into the sandbox so the direct-launch fence
    // can self-invoke `--landlock-exec` inside it. If arlen-run is already under
    // `/usr` (the ro-bound base), it is reachable without an extra bind.
    let arlen_run = std::env::current_exe().ok();
    if let Some(exe) = &arlen_run {
        let exe_str = exe.to_string_lossy();
        if !exe_str.starts_with("/usr/") {
            plumbing.push(arlen_confiner::Bind::ReadOnly(
                exe_str.to_string(),
                exe_str.to_string(),
            ));
        }
    }
    let mut env = launch_env(&home, runtime_dir.as_deref(), wayland_display.as_deref());
    // Point the confined app's HTTP client at the egress proxy (reached at the
    // netns's mapped-loopback gateway). A raw dial that ignores these still hits
    // route-absence, so this is the cooperative path, not the boundary.
    if let Some(port) = proxy_port {
        let url = netns::proxy_env_url(port);
        for key in ["http_proxy", "https_proxy", "all_proxy", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
            env.insert(key.to_string(), url.clone());
        }
    }

    let confinement = match spawn::build_confinement(
        std::path::Path::new("/usr"),
        &inputs.app_dirs,
        &inputs.masked_dirs,
        env,
        inputs.network,
        plumbing,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("arlen-run: confinement setup for {}: {e}", args.app_id);
            return ExitCode::from(exit::CONFINE_SETUP);
        }
    };

    // Ensure the app's own state dirs exist so their Landlock write grant is
    // expressible (a missing writable path is otherwise skipped, leaving the
    // app unable to write its own state). Created mode 0700 (owner-only: an
    // app's private state is not world-readable); best-effort, a failure here is
    // not fatal (the grant is simply dropped for that dir).
    use std::os::unix::fs::DirBuilderExt;
    for dir in &app_state_dirs(&home, &args.app_id) {
        let _ = std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir);
    }

    // Create the per-launch cgroup so the child can join it and the tree can be
    // reaped with one kill. A system without delegated cgroup v2 (some dev
    // setups) is not fatal: the cgroup is a reaping/attribution aid, not a
    // security boundary, so on failure the launch proceeds without it (bwrap's
    // pid-namespace + --die-with-parent still tear the tree down).
    // SAFETY: getpid only reads the launcher's pid.
    let launch_pid = unsafe { libc::getpid() } as u32;
    let uid = unsafe { libc::getuid() };
    let cgroup = match cgroup::Cgroup::create(uid, &args.app_id, launch_pid) {
        Ok(cg) => Some(cg),
        Err(e) => {
            eprintln!("arlen-run: no per-launch cgroup ({e}); reaping falls back to bwrap");
            None
        }
    };
    let cgroup_procs = cgroup.as_ref().map(cgroup::Cgroup::procs_path);

    // The third confinement layer (GAP-6): compile the per-app seccomp allowlist
    // and hand it to bwrap via --seccomp. A filter that cannot be built means the
    // confinement would be a layer short, so refuse the launch (fail-closed),
    // never run the app without it.
    let seccomp_bpf = match seccomp::app_filter_bytes() {
        Ok(bpf) => bpf,
        Err(e) => {
            eprintln!("arlen-run: cannot build the seccomp filter ({e}); refusing to launch");
            return ExitCode::from(exit::CONFINE_SETUP);
        }
    };

    let result = if filtered {
        // The filtered launch wraps bwrap in the route-absent pasta namespace and
        // delivers the seccomp through the wrapper file; the proxy env above is
        // already set. The egress guard (held above) keeps the proxy serving. The
        // in-sandbox Landlock fence is not applied here (this path keeps its own
        // model: bwrap's mount namespace + seccomp); folding the fence in is a
        // follow-up.
        let argv = spawn::bwrap_argv(&confinement, &args.program);
        spawn::spawn_filtered_and_wait(&argv, &inputs.app_dirs, cgroup_procs, seccomp_bpf)
    } else {
        // The direct launch runs the app under the in-sandbox Landlock fence: bwrap
        // execs `arlen-run --landlock-exec <app writable dirs> -- <app>`, which
        // fences the app to its own dirs (read the pruned bwrap view, write only
        // those dirs) then execs it - so the fence confines the app after bwrap's
        // setup. If arlen-run's own path is unknown, the app runs directly and
        // bwrap's mount namespace remains the filesystem confinement (fail-safe).
        // The `--landlock-exec` wrapper protocol uses a literal `--` to separate the
        // writable dirs from the program, so no writable dir may itself be `--`.
        // `build_confinement` (above) already rejected any non-absolute app_dir, so
        // this holds by construction; assert it so a future refactor cannot silently
        // re-open a delimiter-confusion gap.
        debug_assert!(
            !inputs.app_dirs.iter().any(|d| d.as_os_str() == "--"),
            "an app writable dir must never be the wrapper delimiter"
        );
        let program = match &arlen_run {
            Some(exe) => landlock_exec::landlock_exec_program(
                &exe.to_string_lossy(),
                &inputs.app_dirs,
                &args.program,
            ),
            None => {
                eprintln!(
                    "arlen-run: {}: could not resolve arlen-run's own path; \
                     the in-sandbox Landlock layer is not applied (bwrap mount \
                     namespace + seccomp still confine the app)",
                    args.app_id
                );
                args.program.clone()
            }
        };
        let argv = spawn::bwrap_argv(&confinement, &program);
        spawn::spawn_and_wait(&argv, &inputs.app_dirs, cgroup_procs, Some(seccomp_bpf))
    };

    // Reap the subtree (kills any process the app left behind), then the leaf is
    // removed when `cgroup` drops.
    if let Some(cg) = &cgroup {
        cg.kill_all();
    }

    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("arlen-run: failed to spawn {}: {e}", args.app_id);
            ExitCode::from(exit::SPAWN)
        }
    }
}

/// The app's own state directories, always part of its writable set. The
/// launcher creates these before spawning so their write grant is always
/// expressible under Landlock.
#[cfg(target_os = "linux")]
fn app_state_dirs(home: &std::path::Path, app_id: &str) -> Vec<PathBuf> {
    vec![
        home.join(".local/share/arlen/apps").join(app_id),
        home.join(".config/arlen/apps").join(app_id),
        home.join(".cache/arlen/apps").join(app_id),
    ]
}

/// The minimal explicit environment for the confined app. `bwrap --clearenv`
/// wipes the ambient environment, so only these are set: the in-sandbox home,
/// the runtime dir and Wayland display (for the bound sockets), a fixed PATH,
/// and the locale passthrough. The ambient environment is never forwarded.
#[cfg(target_os = "linux")]
fn launch_env(
    home: &std::path::Path,
    runtime_dir: Option<&std::path::Path>,
    wayland_display: Option<&str>,
) -> std::collections::BTreeMap<String, String> {
    let mut env = std::collections::BTreeMap::new();
    if let Some(h) = home.to_str() {
        env.insert("HOME".to_string(), h.to_string());
    }
    env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
    if let Some(rt) = runtime_dir.and_then(|p| p.to_str()) {
        env.insert("XDG_RUNTIME_DIR".to_string(), rt.to_string());
    }
    if let Some(wl) = wayland_display {
        env.insert("WAYLAND_DISPLAY".to_string(), wl.to_string());
    }
    for key in ["LANG", "LC_ALL", "LC_CTYPE"] {
        if let Ok(v) = std::env::var(key) {
            env.insert(key.to_string(), v);
        }
    }
    env
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
