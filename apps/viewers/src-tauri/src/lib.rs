//! The Arlen Quick Look viewer Tauri shell: wires the sandboxed decode host into a
//! window so media opens in a real, isolated viewer rather than the browser
//! (`quickview-plan.md`). The host detects each file's format and spawns its decoder
//! in a bwrap jail; this shell exposes that as the `decode_image` / `probe_audio`
//! commands the frontend calls, and only validated rasters/metadata cross back.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The decoded raster handed to the frontend: 8-bit RGBA, row-major. A serializable
/// projection of the host's render-only `DecodedImage` (the core type stays free of
/// a wire derive).
#[derive(Serialize)]
pub struct DecodedImageDto {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA.
    pub rgba: Vec<u8>,
}

/// Audio metadata handed to the frontend (a serializable projection of the host's
/// `AudioInfo`).
#[derive(Serialize)]
pub struct AudioInfoDto {
    /// The codec short name (e.g. "flac", "mp3", "vorbis").
    pub codec: String,
    /// Samples per second per channel.
    pub sample_rate: u32,
    /// Channel count (1 = mono, 2 = stereo, ...).
    pub channels: u16,
    /// Duration in milliseconds, when the container declares it.
    pub duration_ms: Option<u64>,
}

/// Where the sandboxed decode-worker binaries live: `ARLEN_VIEWERS_WORKER_DIR` if set
/// (the dev/dist override), else the directory of the running viewer binary (the
/// workers ship beside it). The host spawns the per-format worker from here.
fn worker_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ARLEN_VIEWERS_WORKER_DIR") {
        return PathBuf::from(dir);
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Decode an image file in the sandbox and return its RGBA raster.
#[tauri::command]
fn decode_image(path: String) -> Result<DecodedImageDto, String> {
    let dir = worker_dir();
    let decoded = arlen_viewers_host::decode_image_path(&dir.to_string_lossy(), Path::new(&path))?;
    Ok(DecodedImageDto {
        width: decoded.width,
        height: decoded.height,
        rgba: decoded.rgba,
    })
}

/// Probe an audio file in the sandbox and return its metadata (no full decode).
#[tauri::command]
fn probe_audio(path: String) -> Result<AudioInfoDto, String> {
    let dir = worker_dir();
    let info = arlen_viewers_host::probe_audio_path(&dir.to_string_lossy(), Path::new(&path))?;
    Ok(AudioInfoDto {
        codec: info.codec,
        sample_rate: info.sample_rate,
        channels: info.channels,
        duration_ms: info.duration_ms,
    })
}

/// Resolve an XDG base dir from `env_var`, else `$HOME/<fallback>`.
fn xdg_base(env_var: &str, fallback: &str) -> PathBuf {
    if let Some(v) = std::env::var_os(env_var) {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(fallback)
}

/// Handle the `--register-default-handler` / `--unregister-default-handler`
/// subcommands: register THIS viewer binary as the default xdg handler for the media
/// MIME types (so image/audio open in the viewer, not the browser), or revert it.
/// Returns `true` if a subcommand ran (so `main` exits instead of opening a window).
///
/// The xdg mimeapps list is USER-GLOBAL (`~/.config/mimeapps.list`) - it affects
/// EVERY desktop session until reverted, not just a dev session - so the register
/// backs the list up first and the unregister restores that backup. For dogfooding,
/// reversible by design.
pub fn handle_default_handler_args() -> bool {
    let register = std::env::args().any(|a| a == "--register-default-handler");
    let unregister = std::env::args().any(|a| a == "--unregister-default-handler");
    if !register && !unregister {
        return false;
    }

    let apps_dir = xdg_base("XDG_DATA_HOME", ".local/share").join("applications");
    let mimeapps = xdg_base("XDG_CONFIG_HOME", ".config").join("mimeapps.list");
    let backup = PathBuf::from(format!("{}.arlen-viewer-bak", mimeapps.display()));
    let desktop = apps_dir.join(arlen_viewers_host::mimeapps::DESKTOP_FILE);

    if register {
        if mimeapps.exists() && !backup.exists() {
            if let Err(e) = std::fs::copy(&mimeapps, &backup) {
                eprintln!("could not back up {}: {e}", mimeapps.display());
                return true;
            }
        }
        let exec = std::env::current_exe()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "arlen-viewers".to_string());
        match arlen_viewers_host::mimeapps::register_default_handler(&apps_dir, &mimeapps, &exec) {
            Ok(()) => println!(
                "registered the Arlen Viewer as the default handler for image + audio types.\n\
                 NB ~/.config/mimeapps.list is user-global (every session) until reverted; backed up to {}.\n\
                 revert with: arlen-viewers --unregister-default-handler",
                backup.display()
            ),
            Err(e) => eprintln!("failed to register the default handler: {e}"),
        }
    } else {
        if backup.exists() {
            match std::fs::copy(&backup, &mimeapps).and_then(|_| std::fs::remove_file(&backup)) {
                Ok(_) => println!("restored {} from the backup", mimeapps.display()),
                Err(e) => eprintln!("failed to restore {}: {e}", mimeapps.display()),
            }
        } else {
            eprintln!("no Arlen Viewer backup found; left {} unchanged", mimeapps.display());
        }
        let _ = std::fs::remove_file(&desktop);
        println!("removed the Arlen Viewer .desktop entry");
    }
    true
}

/// Tauri entry point (invoked from `main.rs`).
pub fn run() {
    env_logger::init();
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .invoke_handler(tauri::generate_handler![decode_image, probe_audio])
        .run(tauri::generate_context!())
        .expect("error while running arlen-viewers");
}
