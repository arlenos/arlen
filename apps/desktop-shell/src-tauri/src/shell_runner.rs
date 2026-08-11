/// Shell command execution for Waypointer.
///
/// Runs commands via `sh -c` or inside a terminal emulator.

use crate::app_index::AppIndex;

/// Executes a shell command.
///
/// When `in_terminal` is false, runs detached via `sh -c`.
/// When `in_terminal` is true, finds the best terminal emulator and
/// launches the command inside it.
///
/// **A failure is told, not only logged.** Both branches used to spawn from a
/// detached thread and drop the result - the terminal one into a `log::error!`,
/// the `sh -c` one into a `let _ =` that discarded it outright. The user has just
/// typed a command into the launcher and pressed Enter, so a spawn that fails
/// shows as the overlay closing and nothing happening: the same silent stop the
/// app-launch path next door already reports with a toast. Realistic rather than
/// theoretical - `find_terminal` can resolve a terminal this machine does not
/// have, and then every `>` command in the Waypointer fails this way.
///
/// The spawn is inline for the same reason it is in `open_harness_session`:
/// `Command::spawn` forks and execs without waiting on the child, so the thread
/// was never carrying work, only the error.
#[tauri::command]
pub fn execute_shell_command(
    app: tauri::AppHandle,
    index: tauri::State<AppIndex>,
    command: String,
    in_terminal: bool,
) {
    if command.is_empty() {
        return;
    }
    let failed = |e: std::io::Error| {
        log::error!("shell_runner: spawn failed: {e}");
        crate::quick_actions::emit_toast(
            &app,
            crate::quick_actions::ToastKind::Error,
            format!("The command did not run: {e}"),
        );
    };

    if in_terminal {
        let (bin, args) = build_terminal_command(&index, &command);
        let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
        log::info!(
            "shell_runner: spawning {:?} {:?} (WAYLAND_DISPLAY={}, DISPLAY='')",
            bin, args, wayland_display,
        );
        match std::process::Command::new(&bin)
            .args(&args)
            .env("WAYLAND_DISPLAY", &wayland_display)
            .env("DISPLAY", "")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => log::info!("shell_runner: launched in terminal"),
            Err(e) => failed(e),
        }
    } else {
        log::info!("shell_runner: sh -c {:?}", command);
        if let Err(e) = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            failed(e);
        }
    }
}

/// Builds the full (binary, args) vector for running a command in a terminal.
fn build_terminal_command(index: &AppIndex, command: &str) -> (String, Vec<String>) {
    let terminal = find_terminal(index);
    log::info!("shell_runner: resolved terminal={:?}", terminal);

    // xdg-terminal-exec handles everything itself.
    if terminal == "xdg-terminal-exec" {
        return (
            terminal,
            vec!["sh".into(), "-c".into(), command.into()],
        );
    }

    let bin_name = terminal.rsplit('/').next().unwrap_or(&terminal);
    let args = match bin_name {
        // kitty: kitty -- sh -c 'command'
        "kitty" => vec![
            "--".into(), "sh".into(), "-c".into(), command.into(),
        ],
        // foot: foot -- sh -c 'command'
        "foot" => vec![
            "--".into(), "sh".into(), "-c".into(), command.into(),
        ],
        // alacritty: alacritty -e sh -c 'command'
        "alacritty" => vec![
            "-e".into(), "sh".into(), "-c".into(), command.into(),
        ],
        // gnome-terminal: gnome-terminal -- sh -c 'command'
        "gnome-terminal" => vec![
            "--".into(), "sh".into(), "-c".into(), command.into(),
        ],
        // konsole: konsole -e sh -c 'command'
        "konsole" => vec![
            "-e".into(), "sh".into(), "-c".into(), command.into(),
        ],
        // wezterm: wezterm start -- sh -c 'command'
        "wezterm" | "wezterm-gui" => vec![
            "start".into(), "--".into(), "sh".into(), "-c".into(), command.into(),
        ],
        // xterm and generic fallback: -e sh -c 'command'
        _ => vec![
            "-e".into(), "sh".into(), "-c".into(), command.into(),
        ],
    };

    (terminal, args)
}

/// Finds the best terminal emulator.
///
/// Priority:
/// 1. $TERMINAL environment variable
/// 2. xdg-terminal-exec (freedesktop standard)
/// 3. App index (TerminalEmulator category)
/// 4. Hardcoded known terminals in PATH
fn find_terminal(index: &AppIndex) -> String {
    // 1. $TERMINAL env var.
    if let Ok(t) = std::env::var("TERMINAL") {
        if !t.is_empty() && which(&t) {
            log::info!("shell_runner: using $TERMINAL={t}");
            return t;
        }
    }

    // 2. xdg-terminal-exec.
    if which("xdg-terminal-exec") {
        log::info!("shell_runner: using xdg-terminal-exec");
        return "xdg-terminal-exec".into();
    }

    // 3. App index: first TerminalEmulator.
    {
        let apps = index.lock().unwrap();
        for app in apps.iter() {
            if app.categories.iter().any(|c| c == "TerminalEmulator") {
                if let Some(bin) = app.exec.split_whitespace().next() {
                    if which(bin) {
                        log::info!("shell_runner: from app index: {bin}");
                        return bin.to_string();
                    }
                }
            }
        }
    }

    // 4. Known terminals by preference.
    let known = [
        "kitty", "foot", "alacritty", "wezterm", "wezterm-gui",
        "gnome-terminal", "konsole", "xfce4-terminal", "xterm",
    ];
    for name in &known {
        if which(name) {
            log::info!("shell_runner: hardcoded fallback: {name}");
            return name.to_string();
        }
    }

    log::warn!("shell_runner: no terminal found, falling back to xterm");
    "xterm".into()
}

/// Opens a URL with xdg-open.
#[tauri::command]
pub fn open_url(url: String) {
    if url.is_empty() {
        return;
    }
    log::info!("shell_runner: xdg-open {:?}", url);
    std::thread::spawn(move || {
        let _ = std::process::Command::new("xdg-open")
            // `--` first: a name may legally begin with a dash and xdg-open parses a
            // leading-dash argument as its own options ("error: unexpected argument
            // '-z' found / tip: to pass '-z' as a value, use '-- -z'"). The files app
            // is safe by construction because `abs` guarantees a leading slash; these
            // callers pass the string through as it arrives.
            .arg("--")
            .arg(&url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    });
}

/// Checks if a binary exists in PATH.
fn which(name: &str) -> bool {
    // Handle absolute paths.
    if name.starts_with('/') {
        return std::path::Path::new(name).is_file();
    }
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(name).is_file())
        })
        .unwrap_or(false)
}
