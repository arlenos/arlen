/// Shell command execution for Waypointer.
///
/// Runs commands via `sh -c` or inside a terminal emulator.

use crate::app_index::AppIndex;

/// Executes a shell command.
///
/// When `in_terminal` is false, runs detached via `sh -c`.
/// When `in_terminal` is true, finds the best terminal emulator and
/// launches the command inside it.
#[tauri::command]
pub fn execute_shell_command(
    index: tauri::State<AppIndex>,
    command: String,
    in_terminal: bool,
) {
    if command.is_empty() {
        return;
    }

    if in_terminal {
        let (bin, args) = build_terminal_command(&index, &command);
        let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
        log::info!(
            "shell_runner: spawning {:?} {:?} (WAYLAND_DISPLAY={}, DISPLAY='')",
            bin, args, wayland_display,
        );
        std::thread::spawn(move || {
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
                Err(e) => log::error!("shell_runner: spawn failed: {e}"),
            }
        });
    } else {
        log::info!("shell_runner: sh -c {:?}", command);
        std::thread::spawn(move || {
            let _ = std::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        });
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
