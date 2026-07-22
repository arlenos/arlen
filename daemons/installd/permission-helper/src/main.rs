/// Arlen Permission Helper -- root D-Bus service.
///
/// Provides `org.arlen.PermissionHelper1` for writing permission profiles
/// to `/var/lib/arlen/permissions/`. Only authorized callers (installd,
/// settings) may invoke methods.
///
/// See `docs/architecture/permission-system.md`.

mod apt_enroll;
mod apt_hook;
mod dbus;
mod identity;
mod profile;

use zbus::connection;

/// Where the curated starting profiles are installed.
const CURATED_DIR: &str = "/usr/share/arlen/profiles";

/// The curated-profile directory, with a debug-only override so the hook can be
/// exercised end to end without writing system paths. Release pins the installed
/// location for the same reason `profile::base_dir` does: an env misconfiguration
/// must not be able to point the enrolment at attacker-chosen grants.
fn curated_dir() -> std::path::PathBuf {
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("ARLEN_CURATED_PROFILES_DIR") {
        return std::path::PathBuf::from(dir);
    }
    std::path::PathBuf::from(CURATED_DIR)
}

/// Run the one-shot apt enrolment: read a `DPkg::Pre-Install-Pkgs` stream on
/// stdin and write the curated profile of every matched package into the system
/// tier, for every human uid.
///
/// **Always exits 0.** apt aborts the package operation when a hook fails, and
/// refusing to install software is a worse outcome than leaving one package
/// unconfined - the miss is logged and learning mode (§E9) still covers it. The
/// parse itself is installd's `apt_hook`; this binary is the privileged half,
/// because the hook runs as root inside apt with no session bus to reach
/// installd through.
fn run_apt_hook() {
    use std::io::Read;

    let mut stream = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut stream) {
        tracing::warn!("apt-enroll: cannot read the hook stream: {e}");
        return;
    }
    let matched = match apt_hook::match_enrollments(&stream, &curated_dir()) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("apt-enroll: cannot parse the hook stream: {e}");
            return;
        }
    };
    if matched.is_empty() {
        tracing::info!("apt-enroll: no installed package has a curated profile");
        return;
    }
    let passwd = std::fs::read_to_string("/etc/passwd").unwrap_or_default();
    let uids = apt_enroll::human_uids(&passwd);
    if uids.is_empty() {
        tracing::warn!("apt-enroll: no human accounts to enroll for");
        return;
    }
    // Hand the launcher rewrite to the post-install stage: the .desktop files
    // do not exist yet during Pre-Install-Pkgs.
    let enrolled: Vec<String> = matched.iter().map(|(p, _)| p.clone()).collect();
    if let Err(e) = apt_enroll::record_pending(&state_dir(), &enrolled) {
        tracing::warn!("apt-enroll: cannot record pending launchers: {e}");
    }
    for outcome in apt_enroll::enroll_matched(&matched, &uids, &profile::base_dir()) {
        match outcome {
            apt_enroll::Enrolled::Written { package, paths } => {
                tracing::info!("apt-enroll: confined {package} for {} uid(s)", paths.len());
            }
            apt_enroll::Enrolled::Failed { package, reason } => {
                // Loud, because the package installed and is running unconfined.
                tracing::warn!("apt-enroll: {package} left unconfined: {reason}");
            }
        }
    }
}

/// Where the two apt stages hand off. Under the helper's own state root, so it
/// is root-owned like everything else it writes.
fn state_dir() -> std::path::PathBuf {
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("ARLEN_ENROLL_STATE_DIR") {
        return std::path::PathBuf::from(dir);
    }
    std::path::PathBuf::from("/var/lib/arlen")
}

/// The launcher-override directory, with the same debug-only override the other
/// paths carry so the stage is testable without writing system directories.
fn override_dir() -> std::path::PathBuf {
    #[cfg(debug_assertions)]
    if let Ok(dir) = std::env::var("ARLEN_OVERRIDE_APPLICATIONS_DIR") {
        return std::path::PathBuf::from(dir);
    }
    std::path::PathBuf::from(apt_enroll::OVERRIDE_DIR)
}

/// Run the post-install stage: rewrite the launchers of everything the
/// pre-install stage enrolled, now that dpkg has unpacked them.
///
/// Also always exits 0. This runs from `DPkg::Post-Invoke`, which fires on every
/// apt run including ones that enrolled nothing.
fn run_apt_post_install() {
    let packages = apt_enroll::take_pending(&state_dir());
    if packages.is_empty() {
        return;
    }
    let dir = override_dir();
    for package in packages {
        let listing = match std::process::Command::new("dpkg").arg("-L").arg(&package).output() {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).into_owned(),
            Ok(out) => {
                tracing::warn!(
                    "apt-enroll: dpkg -L {package} failed ({}); launcher not confined",
                    out.status
                );
                continue;
            }
            Err(e) => {
                tracing::warn!("apt-enroll: cannot run dpkg -L {package}: {e}");
                continue;
            }
        };
        let files = apt_hook::classify_package_files(&listing);
        match apt_enroll::write_launcher_overrides(&package, &files.desktop_entries, &dir) {
            Ok(written) if written.is_empty() => {
                tracing::info!("apt-enroll: {package} ships no launcher; profile only");
            }
            Ok(written) => {
                tracing::info!("apt-enroll: {package} launches confined ({} entries)", written.len());
            }
            // Loud: the package is installed with a profile but still launches
            // unconfined, which is the state most likely to be mistaken for done.
            Err(e) => tracing::warn!("apt-enroll: {package} launcher not confined: {e}"),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("arlen_permission_helper=info".parse()?),
        )
        .init();

    // apt invokes this binary as a hook, not as the bus service.
    if std::env::args().any(|a| a == "--apt-hook") {
        run_apt_hook();
        return Ok(());
    }
    if std::env::args().any(|a| a == "--apt-post-install") {
        run_apt_post_install();
        return Ok(());
    }

    tracing::info!("starting permission helper");

    let helper = dbus::PermissionHelper;

    let _conn = connection::Builder::system()?
        .name("org.arlen.PermissionHelper1")?
        .serve_at("/org/arlen/PermissionHelper1", helper)?
        .build()
        .await?;

    tracing::info!("D-Bus service ready on org.arlen.PermissionHelper1");

    // Run until SIGTERM.
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}
