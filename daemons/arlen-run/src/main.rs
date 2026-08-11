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
// The Tier-1 identity-stamp helpers. The pure format-critical pieces land first;
// the spawn-path wiring that calls them is a following slice, so allow dead_code
// until it does (mechanism before trigger).
#[cfg_attr(not(test), allow(dead_code))]
mod stamp;
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

/// Whether `app_id` is safe to put into a profile path AND a cgroup name: a
/// non-empty `[A-Za-z0-9._-]` id, no `..`, no leading or trailing dot. It lands in
/// both a filesystem path and a cgroup leaf name, so it is validated before either.
/// The shape is a safety rule, not a naming convention - see the body.
fn valid_app_id(app_id: &str) -> bool {
    // The charset and traversal rules come from the profile loader, because this
    // launcher must be able to run every app the loader can address - restating
    // them here is how it came to reject `org.gnome.Calculator` while the rest of
    // the system accepted it.
    //
    // There is no reverse-domain requirement. It used to be here, and it was a
    // syntax proxy for a rule we hold properly elsewhere: the danger was never
    // punctuation, it was a foreign package claiming `settings` and inheriting a
    // first-party profile, which `validate_manifest` refuses by name. Measured on
    // 9 August, the dot refused 1415 of the 2273 authored third-party profiles and
    // 155 of 230 real `.desktop` entries, because desktop ids in the wild are
    // `1password` and `slic3r`. We do not get to define the identifier someone
    // else's software ships with. Our own apps are `dev.arlen.<app>` by choice.
    arlen_permissions::is_valid_app_id(app_id)
}

/// The parsed launch request.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    /// The app id (validated).
    app_id: String,
    /// Optional override for the directory `{app_id}.toml` is read from.
    profile_root: Option<PathBuf>,
    /// The program and its argv (everything after `--`).
    program: Vec<String>,
    /// Run with no trace over a single untrusted file (§E10) instead of the app's
    /// permission profile. `None` is the ordinary profiled launch.
    ephemeral_file: Option<PathBuf>,
}

/// Parse `arlen-run --app-id <id> [--profile-root <dir>] [--ephemeral <file>] --
/// <program> [args...]` from the argument list (excluding the binary name).
/// Returns the parsed request, or the exit code to fail with: an unknown flag, a
/// missing/invalid `--app-id`, a missing `--`, or an empty program is `BAD_ARGS`.
///
/// `--ephemeral` takes the ONE untrusted file the launch may see, and requires an
/// absolute path - the confiner refuses a relative bind source anyway, but failing
/// here names the argument instead of surfacing later as a confinement-setup
/// error. It is rejected alongside `--profile-root`, which points at a profile an
/// ephemeral launch never reads: accepting both would let a caller believe a
/// profile was in force when nothing loaded it.
fn parse_args(args: &[String]) -> Result<Args, u8> {
    let mut app_id: Option<String> = None;
    let mut profile_root: Option<PathBuf> = None;
    let mut ephemeral_file: Option<PathBuf> = None;
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
            "--ephemeral" => {
                let value = args.get(i + 1).ok_or(exit::BAD_ARGS)?;
                let path = PathBuf::from(value);
                if !path.is_absolute() {
                    return Err(exit::BAD_ARGS);
                }
                ephemeral_file = Some(path);
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
                if ephemeral_file.is_some() && profile_root.is_some() {
                    return Err(exit::BAD_ARGS);
                }
                return Ok(Args {
                    app_id,
                    profile_root,
                    program,
                    ephemeral_file,
                });
            }
            _ => return Err(exit::BAD_ARGS),
        }
    }
    // No `--` separator: there is no program to run.
    Err(exit::BAD_ARGS)
}

/// The tmpfs mounts an ephemeral launch gets, which are also the only places it
/// can write - so they are its Landlock writable set. Kept next to the branch that
/// uses them because they must stay identical to `ephemeral_profile`'s `tmpfs`:
/// fencing a directory the sandbox does not have, or missing one it does, either
/// breaks the launch or leaves a writable path unfenced.
#[cfg(target_os = "linux")]
const EPHEMERAL_WRITABLE: &[&str] = &["/home", "/tmp", "/run"];

/// Run `program` over ONE untrusted file with no trace (§E10).
///
/// No permission profile, no app dirs, no cgroup and no egress setup, because
/// there is no profile to derive any of them from - and no audit, which is the
/// point: nothing about this launch is recorded. The file enters as a single
/// read-only bind at a fixed path rather than a directory grant, `/home` is a
/// fresh tmpfs so the user's home is not merely unbound but masked, and network is
/// off outright.
///
/// The base platform is `/usr`, the same root the profiled path treats as the
/// platform, rather than the whole host filesystem: read-only or not, an untrusted
/// document's viewer has no business enumerating `/etc`, `/var` and `/root`. The
/// merged-`/usr` compat paths are bound alongside so a dynamically-linked viewer
/// finds its ELF interpreter.
///
/// **The consequence for callers: the program must live under `/usr`.** Nothing
/// else is bound, so a viewer installed at `~/.local/lib/arlen/libexec/...` - or
/// run out of a dev build tree - is simply not present inside and exec fails with
/// "No such file or directory". Whatever `Exec=` the untrusted-content MIME
/// handler registers has to resolve within a `/usr`-only view.
#[cfg(target_os = "linux")]
fn run_ephemeral(app_id: &str, file: &std::path::Path, program: &[String]) -> ExitCode {
    // The merged-`/usr` root-level compat paths that actually exist here, so a
    // dynamically-linked viewer finds its ELF interpreter inside the sandbox.
    // Shared with the confiner rather than restated: the command path had this
    // list missing entirely, which is how `run_command` came to be unable to run
    // any dynamically linked program.
    let compat_owned = arlen_confiner::merged_usr_compat_roots();
    let compat: Vec<&std::path::Path> =
        compat_owned.iter().map(std::path::Path::new).collect();
    let confinement = match arlen_confiner::ephemeral_profile(
        std::path::Path::new("/usr"),
        file,
        arlen_confiner::NetworkPolicy::None,
        &compat,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("arlen-run: ephemeral confinement for {app_id}: {e}");
            return ExitCode::from(exit::CONFINE_SETUP);
        }
    };

    // Same fail-closed rule as the profiled path: a filter that cannot be built
    // would leave the confinement a layer short, so refuse rather than run.
    let seccomp_bpf = match seccomp::app_filter_bytes() {
        Ok(bpf) => bpf,
        Err(e) => {
            eprintln!("arlen-run: cannot build the seccomp filter ({e}); refusing to launch");
            return ExitCode::from(exit::CONFINE_SETUP);
        }
    };

    let writable: Vec<PathBuf> = EPHEMERAL_WRITABLE.iter().map(PathBuf::from).collect();
    // The in-sandbox Landlock wrapper self-invokes arlen-run from INSIDE the
    // sandbox, so it only works when arlen-run is reachable there. The profiled
    // path binds the binary in when it lives outside `/usr`; here `/usr` is the
    // only bind, so an arlen-run outside it simply cannot be re-executed. Fall
    // back to launching the program directly, exactly as the profiled path does
    // when it cannot resolve its own binary: bwrap's mount namespace and the
    // seccomp filter still confine, and the ephemeral view has no app dirs to
    // fence anyway.
    let exe = std::env::current_exe().ok();
    let program = match exe.as_deref().map(|e| e.to_string_lossy().into_owned()) {
        Some(exe) if exe.starts_with("/usr/") => {
            landlock_exec::landlock_exec_program(&exe, &writable, program)
        }
        _ => program.to_vec(),
    };
    let argv = spawn::bwrap_argv(&confinement, &program);
    match spawn::spawn_and_wait(&argv, &writable, None, Some(seccomp_bpf), app_id) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("arlen-run: failed to spawn the ephemeral {app_id}: {e}");
            ExitCode::from(exit::SPAWN)
        }
    }
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

    // The no-trace path (§E10) reads no profile at all, so it branches out before
    // the load rather than threading an Option through everything below.
    if let Some(file) = args.ephemeral_file.clone() {
        return run_ephemeral(&args.app_id, &file, &args.program);
    }

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
            // The error already names the app (`profile not found for <id>`),
            // and this line is read by a person now rather than only by a log:
            // the shell surfaces it verbatim when a confined launch is refused,
            // so a prefix that repeats the id reads as a stutter on screen.
            eprintln!("arlen-run: {e}");
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
        &inputs.read_only_dirs,
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
            eprintln!(
                "arlen-run: warning: no per-launch cgroup ({e}); reaping falls back to bwrap"
            );
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

    // Run the app under the in-sandbox Landlock fence on BOTH paths: bwrap execs
    // `arlen-run --landlock-exec <app writable dirs> -- <app>`, which fences the app
    // to its own dirs (plus the standard writable devices) after bwrap's mount +
    // userns setup, then execs it - a filesystem defense-in-depth layer independent
    // of bwrap's mount namespace. If arlen-run's own path is unknown, the app runs
    // directly and bwrap's mount namespace remains the filesystem confinement
    // (fail-safe). The wrapper uses a literal `--` to separate the writable dirs
    // from the program, so no writable dir may itself be `--`; `build_confinement`
    // above already rejected any non-absolute app_dir, so this holds by
    // construction (asserted against a future refactor).
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
                "arlen-run: {}: could not resolve arlen-run's own path; the \
                 in-sandbox Landlock layer is not applied (bwrap mount namespace \
                 + seccomp still confine the app)",
                args.app_id
            );
            args.program.clone()
        }
    };
    let argv = spawn::bwrap_argv(&confinement, &program);
    let result = if filtered {
        // The filtered launch additionally wraps bwrap in the route-absent pasta
        // namespace and delivers the seccomp through the wrapper file; the proxy env
        // above is already set, and the egress guard (held above) keeps the proxy
        // serving. The fence composes: the seccomp allowlist admits the landlock
        // syscalls, and pasta's netns is orthogonal to the filesystem fence.
        spawn::spawn_filtered_and_wait(&argv, &inputs.app_dirs, cgroup_procs, seccomp_bpf)
    } else {
        spawn::spawn_and_wait(&argv, &inputs.app_dirs, cgroup_procs, Some(seccomp_bpf), &args.app_id)
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
    // Point the XDG data home at the parent of the app's own granted directory.
    //
    // A Tauri app's `appDataDir()` is `$XDG_DATA_HOME/<bundle identifier>`, and
    // its webview keeps WebKitCache, CacheStorage and hsts-storage.sqlite there.
    // With the ambient value that is `~/.local/share/<id>`, which is NOT granted
    // and cannot even be created inside the sandbox - measured, and it would have
    // broken every app the image ships, the ones WITH a profile included.
    //
    // Redirecting here rather than granting that path keeps the profile grammar
    // small (no implicit per-app grant every profile has to remember, whose
    // omission fails inside the webview, the least legible place we have) and puts
    // the state in a namespace we own, so uninstall can clean it and the privacy
    // surface can name it. It works out to the same directory because the app id
    // IS the bundle identifier: `$XDG_DATA_HOME/<id>` becomes
    // `~/.local/share/arlen/apps/<id>`, which is bound read-write.
    if let Some(d) = home.join(".local/share/arlen/apps").to_str() {
        env.insert("XDG_DATA_HOME".to_string(), d.to_string());
    }
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
    fn the_ephemeral_flag_takes_one_absolute_file() {
        let args = super::parse_args(&[
            "--app-id".into(),
            "org.arlen.Viewer".into(),
            "--ephemeral".into(),
            "/tmp/untrusted.pdf".into(),
            "--".into(),
            "viewer".into(),
        ])
        .expect("parses");
        assert_eq!(
            args.ephemeral_file.as_deref(),
            Some(std::path::Path::new("/tmp/untrusted.pdf"))
        );
    }

    #[test]
    fn a_relative_ephemeral_file_is_refused_by_name() {
        // The confiner would refuse a relative bind source anyway; failing here
        // means the error names the argument instead of arriving as an opaque
        // confinement-setup failure.
        assert_eq!(
            super::parse_args(&[
                "--app-id".into(),
                "org.arlen.Viewer".into(),
                "--ephemeral".into(),
                "untrusted.pdf".into(),
                "--".into(),
                "viewer".into(),
            ]),
            Err(super::exit::BAD_ARGS)
        );
    }

    #[test]
    fn ephemeral_and_a_profile_root_together_are_refused() {
        // An ephemeral launch reads no profile. Accepting both would let a caller
        // believe the profile they pointed at was in force.
        assert_eq!(
            super::parse_args(&[
                "--app-id".into(),
                "org.arlen.Viewer".into(),
                "--profile-root".into(),
                "/tmp/profiles".into(),
                "--ephemeral".into(),
                "/tmp/untrusted.pdf".into(),
                "--".into(),
                "viewer".into(),
            ]),
            Err(super::exit::BAD_ARGS)
        );
    }

    #[test]
    fn an_ordinary_launch_is_not_ephemeral() {
        let args = super::parse_args(&[
            "--app-id".into(),
            "org.gnome.Calculator".into(),
            "--".into(),
            "calc".into(),
        ])
        .expect("parses");
        assert!(args.ephemeral_file.is_none());
    }

    /// The writable set handed to Landlock must be exactly the tmpfs mounts the
    /// ephemeral confinement creates. If they drift, the fence either names a
    /// directory the sandbox does not have or leaves a writable one unfenced.
    #[test]
    fn the_ephemeral_writable_set_matches_the_confinement_tmpfs() {
        let conf = arlen_confiner::ephemeral_profile(
            std::path::Path::new("/usr"),
            std::path::Path::new("/tmp/untrusted.pdf"),
            arlen_confiner::NetworkPolicy::None,
            &[],
        )
        .expect("the profile builds");
        let args = conf.bwrap_args();
        for dir in super::EPHEMERAL_WRITABLE {
            assert!(
                args.windows(2).any(|w| w[0] == "--tmpfs" && w[1] == *dir),
                "{dir} is in the writable set but the confinement does not tmpfs it"
            );
        }
        let tmpfs_count = args.iter().filter(|a| *a == "--tmpfs").count();
        assert_eq!(
            tmpfs_count,
            super::EPHEMERAL_WRITABLE.len(),
            "the confinement tmpfs-es a directory the writable set does not fence"
        );
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
            ".leading",          // leading dot
            "trailing.",         // trailing dot
            "a..b",              // double dot
            "a/b.c",             // separator
            "a b.c",             // space
            "café.app",          // non-ascii
        ] {
            assert!(!valid_app_id(bad), "{bad:?} must be invalid");
        }
    }

    #[test]
    fn a_real_apps_id_can_be_launched() {
        // This launcher must accept every id the profile loader can address, or the
        // app cannot run confined at all. It used to demand lowercase, which
        // excluded most real apps, and then a reverse-domain dot, which excluded
        // 1415 of the 2273 profiles we have authored - `.desktop` files in the wild
        // carry `1password`, not `com.agilebits.1password`.
        for good in [
            "org.gnome.Calculator",
            "app.drey.Biblioteca",
            "com.example.notes",
            "1password",
            "slic3r",
            "gnome-power-manager",
        ] {
            assert!(valid_app_id(good), "{good:?} must be launchable");
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

    /// The id becomes a path component, so a separator is refused at the argument
    /// boundary rather than deeper. A dotless id is NOT invalid: that was a naming
    /// convention, and it is the id most real applications ship with.
    #[test]
    fn rejects_an_invalid_app_id() {
        assert_eq!(
            parse_args(&args(&["--app-id", "a/b", "--", "prog"])),
            Err(exit::BAD_ARGS)
        );
        assert!(parse_args(&args(&["--app-id", "1password", "--", "prog"])).is_ok());
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
