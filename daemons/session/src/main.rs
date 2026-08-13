//! The Arlen graphical session.
//!
//! greetd's `[initial_session]` starts this. It settles the environment, mints the
//! session id, brings up the compositor, waits for its Wayland socket, hands the
//! environment to the `systemd --user` manager so the user services inherit it,
//! reaches `graphical-session.target`, starts the shell, and waits - when the
//! compositor exits, the session ends.
//!
//! The decisions live in the library beside this ([`arlen_session::env`],
//! [`arlen_session::session_id`], [`arlen_session::verify_app`],
//! [`arlen_session::wayland`]) so they are testable without a seat. What is left
//! here is process work: spawning, waiting, and reporting what happened.
//!
//! **This IS the login path**: `/usr/bin/arlen-session` is this binary, and greetd
//! names that exact path. The swap was gated on a boot rather than an argument, and
//! the boot of 13 Aug 11:12 is what authorised it - compositor rendering, shell
//! drawing, the graph ingesting that run's own file, and the AI leg answering.

use std::collections::BTreeMap;
use std::os::fd::AsFd;
use std::process::{Command, Stdio};

use arlen_session::env::{import_list, session_env, MUST_BE_UNSET, WAYLAND_DISPLAY};
use arlen_session::session_id::{session_id, SESSION_ID_VAR};
use arlen_permissions::identity::app_id_for_program;
use arlen_session::stamp::{grants_registrar, COMPOSITOR, SHELL};
use arlen_session::verify_app::requested_app;
use arlen_session::wayland::{wait_for_display, WAIT_STEPS};

/// Where the kernel's DMI driver exposes the SMBIOS fields the boot-verify
/// harness passes in. World-readable, no kernel module needed.
const PRODUCT_SKU: &str = "/sys/class/dmi/id/product_sku";
const PRODUCT_FAMILY: &str = "/sys/class/dmi/id/product_family";

/// Read a sysfs string, or empty when it is not there. Absent is the normal case:
/// QEMU leaves these empty and real hardware has its own values.
fn dmi(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Run a command with its output routed into the journal under `tag`, so a
/// headless boot can be read from the serial console. Returns the child.
fn spawn_logged(tag: &str, program: &str, env: &BTreeMap<String, String>) -> std::io::Result<std::process::Child> {
    let mut cmd = Command::new("systemd-cat");
    cmd.arg(format!("--identifier={tag}")).arg(program);
    for (k, v) in env {
        cmd.env(k, v);
    }
    // Strip the display variables ONLY where the caller has not set them.
    //
    // The two children want opposite things from the same list. The compositor
    // must see neither, or it picks its nested x11/winit backend instead of
    // driving the seat's DRM device - which is what `MUST_BE_UNSET` is for. The
    // shell must see `WAYLAND_DISPLAY`, because that is how it finds the
    // compositor at all.
    //
    // Removing them unconditionally does both jobs wrong in one line, and the boot
    // of 13 Aug is what said so: the compositor came up, the session found its
    // socket, started the shell without the display it had just learned, and the
    // shell panicked in `gtk::rt::init` - "Failed to initialize GTK" - which reads
    // as a graphics-stack problem and is an environment one.
    for var in MUST_BE_UNSET {
        if !env.contains_key(*var) {
            cmd.env_remove(var);
        }
    }
    let child = cmd.stdin(Stdio::null()).spawn()?;
    stamp_child(&child, program);
    Ok(child)
}

/// Tell the identity broker what the session just started.
///
/// This is what makes the session the root of trust in practice rather than in
/// principle: a registrar is admitted because the SESSION ROOT registered it, so
/// the set of things that may stamp identities is exactly what this function has
/// stamped. Nothing here consults a list of names.
///
/// Best-effort, like the launcher's. A broker that is not up must never stop a
/// login: the child then resolves the way it did before, which is the state this
/// improves on rather than replaces. `systemd-cat` EXECS the program, so the pid
/// held here is the program's own once the exec completes.
fn stamp_child(child: &std::process::Child, program: &str) {
    let path_var = std::env::var("PATH").unwrap_or_default();
    let Some(app_id) = app_id_for_program(program, &path_var) else {
        say(&format!(
            "'{program}' resolves to no app id, so it was started unstamped and \
             falls back to the older resolvers"
        ));
        return;
    };
    let Some(pidfd) = arlen_permissions::peer_pidfd::pidfd_open(child.id()) else {
        return;
    };
    if let Err(e) = arlen_permissions::identity_wire::register_identity(
        &arlen_permissions::identity_wire::identity_broker_connect_path(),
        pidfd.as_fd(),
        &app_id,
        grants_registrar(program),
    ) {
        say(&format!("'{app_id}' was not stamped ({e}); it resolves via /proc"));
    }
}

/// Say something on the session's own channel. On this headless appliance the
/// serial console is the only channel there is, so a silent failure is a black
/// screen and nothing else.
fn say(message: &str) {
    let _ = Command::new("systemd-cat")
        .arg("--identifier=arlen-session")
        .arg("printf")
        .arg("%s\n")
        .arg(message)
        .status();
    eprintln!("arlen-session: {message}");
}

fn main() -> std::process::ExitCode {
    let id = session_id(std::env::var(SESSION_ID_VAR).ok());
    let mut env = session_env(&id, &dmi(PRODUCT_FAMILY));

    // Name the identity broker's uid for everything this session will start.
    //
    // Without it the launcher-stamped identity tier is INERT: a daemon refuses to
    // trust any broker it cannot pin to a uid (correctly - the per-user socket path
    // is user-writable, so trusting whoever answers there would be the fail-open
    // this check exists to prevent), falls back to reading the peer's
    // `/proc/<pid>/exe`, and under a hardened unit that read is refused. Measured on
    // the boot of 13 Aug: every peer resolved to nothing and the undo signer
    // rejected each connection with `readlinkat(exe): Permission denied`.
    //
    // The uid is not a number anyone can write down - `sysusers` allocates it - so
    // it is read from the owner of the SYSTEM socket, in a directory only root and
    // the broker may write. Setting it here rather than in sixteen units is what
    // makes it reach the apps too: the import below hands it to the user manager,
    // and every user service and launched app inherits it from there.
    match arlen_permissions::identity_wire::broker_uid_from_system_socket() {
        Some(uid) => {
            env.insert(
                arlen_permissions::identity_wire::IDENTITY_BROKER_UID_ENV.to_string(),
                uid.to_string(),
            );
        }
        None => say(
            "the identity broker's socket is not there yet, so stamped identity stays \
             off for this session and peers resolve via /proc",
        ),
    }

    // The user's folders, named in the user's language, BEFORE anything binds
    // them. Nothing else in the image runs this: a normal desktop gets it from
    // /etc/xdg/autostart, which we do not process. Without it a German install
    // has its files in ~/Dokumente while the launcher confines ~/Documents, and
    // every read under that grant is refused - correctly and invisibly. Absent
    // tool is not a session failure.
    if Command::new("xdg-user-dirs-update").status().is_err() {
        say("xdg-user-dirs-update did not run; user directories keep their previous names");
    }

    let Ok(mut compositor) = spawn_logged("arlen-compositor", COMPOSITOR, &env) else {
        say("the compositor could not be started; there is no session to have");
        return std::process::ExitCode::FAILURE;
    };

    // Wait for it to publish its socket, and capture the name so the shell can
    // connect. Everything graphical below is gated on this.
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
    let display = wait_for_display(
        || {
            std::fs::read_dir(&runtime)
                .map(|d| {
                    d.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default()
        },
        std::thread::sleep,
        WAIT_STEPS,
    );

    if let Some(display) = &display {
        env.insert(WAYLAND_DISPLAY.to_string(), display.clone());
        // Hand the environment to the user manager, so the shell and the app user
        // services inherit it. The list is derived from what was exported, so it
        // cannot fall out of step with it.
        let mut import = Command::new("systemctl");
        import.arg("--user").arg("import-environment");
        for name in import_list(&env) {
            import.arg(name);
        }
        for (k, v) in &env {
            import.env(k, v);
        }
        let _ = import.status();

        // graphical-session.target, which the sequence has always claimed and
        // never reached: units that say `WantedBy=graphical-session.target` (the
        // wallpaper renderer) were installed, enabled and never run. It goes here
        // because the import above is what gives them WAYLAND_DISPLAY.
        let _ = Command::new("systemctl")
            .arg("--user")
            .arg("start")
            .arg("graphical-session.target")
            .status();

        let _ = spawn_logged("arlen-shell", SHELL, &env);

        // The boot-verify hook: one app, named in the SMBIOS SKU, sanitised to the
        // app-name charset and required to resolve to an installed binary - so it
        // can only start a real app, never inject a command.
        if let Some(app) = requested_app(&dmi(PRODUCT_SKU)) {
            if Command::new("sh")
                .arg("-c")
                .arg(format!("command -v {app}"))
                .stdout(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
            {
                say(&format!("launching verify app '{app}'"));
                let _ = spawn_logged("verify-app", &app, &env);
            } else {
                say(&format!("verify app '{app}' is not an installed binary"));
            }
        }
    } else {
        // The compositor never published a socket in the budget, so nothing
        // graphical can start. Say so plainly: a silent black screen otherwise
        // reads as a mystery, and the cause is upstream in the compositor's own
        // init rather than here.
        say(
            "the compositor published no Wayland socket within 10s; the shell will NOT \
             start - see the compositor's journal above for an EGL, xkb or DRM failure",
        );
    }

    // The session lives as long as the compositor does.
    let status = compositor.wait();
    match status {
        Ok(s) if s.success() => std::process::ExitCode::SUCCESS,
        _ => std::process::ExitCode::FAILURE,
    }
}
