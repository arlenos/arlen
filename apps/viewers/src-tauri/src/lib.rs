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
    /// The track title tag, when present (the player falls back to the file name).
    pub title: Option<String>,
    /// The artist tag, when present.
    pub artist: Option<String>,
    /// The waveform envelope: up to 180 peak-normalised bars (0-255). Empty when
    /// the track length is unknown or silent (the player falls back).
    pub peaks: Vec<u8>,
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
        title: info.title,
        artist: info.artist,
        peaks: info.peaks,
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

/// The file path the viewer was launched with (`viewer <path>`, or the `.desktop`
/// `Exec=<bin> %f` when opened from the FM / a double-click). Managed so the
/// frontend can fetch it on mount via [`initial_file`]. `None` when launched with
/// no file (the harness/mock path).
struct InitialFile(Option<String>);

/// The path the viewer was opened on, for the frontend to load on mount. The
/// decode commands take their path from the frontend, so this is how an
/// argv/`%f`-supplied file reaches them.
#[tauri::command]
fn initial_file(state: tauri::State<'_, InitialFile>) -> Option<String> {
    state.0.clone()
}

/// Which viewer surface a file needs: `"image"` or `"audio"` (so the frontend
/// calls `decode_image` vs `probe_audio`). Detects by magic bytes then extension
/// through the shared core, the same detection the decode path uses. An
/// unsupported file is an error the frontend surfaces, never a guess.
#[tauri::command]
fn detect_media_kind(path: String) -> Result<String, String> {
    use std::io::Read;
    let p = Path::new(&path);
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    // A bounded head is enough for magic detection (every signature is well
    // under 64 bytes); extension-only formats resolve from `name`.
    let mut head = [0u8; 64];
    let read = std::fs::File::open(p)
        .and_then(|mut f| f.read(&mut head))
        .map_err(|e| e.to_string())?;
    match arlen_viewers_core::detect(name, &head[..read]) {
        Some(d) => Ok(match d.kind {
            arlen_viewers_core::MediaKind::Image => "image".to_string(),
            arlen_viewers_core::MediaKind::Audio => "audio".to_string(),
        }),
        None => Err("unsupported file format".to_string()),
    }
}

/// The file after or before this one in its folder, for the arrow keys.
///
/// `direction` is `"next"` or `"previous"`. Returns the absolute path of the
/// neighbour, or `None` when there is nowhere to go - a lone picture in its
/// folder, or a file the viewer cannot show. `None` is a real answer here and the
/// frontend leaves the view as it is; returning the current path instead would
/// make the key look like it worked.
///
/// The directory read happens here and the ORDER is decided by the shared core,
/// which is where the rules and their reasons live (same folder, same media
/// kind, a total case-insensitive order, wrapping at both ends). Splitting it
/// that way keeps the part worth testing free of the disk.
#[tauri::command]
fn neighbour_file(path: String, direction: String) -> Result<Option<String>, String> {
    let (dir, name, entries) = listing(&path)?;

    let picked = match direction.as_str() {
        "next" => arlen_viewers_core::navigate::next(&name, &entries),
        "previous" => arlen_viewers_core::navigate::previous(&name, &entries),
        other => return Err(format!("unknown direction: {other}")),
    };

    Ok(picked.map(|n| dir.join(n).to_string_lossy().into_owned()))
}

/// Where the open file sits among its neighbours, as `[index, total]`.
///
/// The counter in the status pill read `1 / 1` for a folder of three pictures,
/// because the frontend had those two numbers hardcoded. Reading the folder is
/// the only way to know them, and this is the same read `neighbour_file` does,
/// so the counter and the arrow keys cannot disagree about what the folder holds.
///
/// `None` when the file is not viewable or has vanished from its own folder;
/// the pill then shows no position rather than an invented one.
#[tauri::command]
fn folder_position(path: String) -> Result<Option<[usize; 2]>, String> {
    let (_, name, entries) = listing(&path)?;
    Ok(arlen_viewers_core::navigate::position(&name, &entries).map(|(i, n)| [i, n]))
}

/// What the filesystem knows about the open file, for the details panel.
///
/// Deliberately only what a `stat` actually answers. The plan calls this panel
/// "EXIF for image, tags/codec for audio, format/streams for video", and the
/// tags and the codec are real - the audio probe reads them - but there is no
/// EXIF parser in this app, so the panel shows no EXIF rather than a section of
/// blanks implying the picture carried none.
#[derive(Serialize)]
pub struct FileFactsDto {
    /// Size on disk in bytes.
    pub size_bytes: u64,
    /// Last modification, milliseconds since the epoch. `None` when the platform
    /// does not report one, which is a real answer and not zero.
    pub modified_ms: Option<i64>,
}

/// `stat` the open file for the details panel.
#[tauri::command]
fn file_facts(path: String) -> Result<FileFactsDto, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("cannot stat the file: {e}"))?;
    let modified_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok());
    Ok(FileFactsDto {
        size_bytes: meta.len(),
        modified_ms,
    })
}

/// Where a deleted file went, and what putting it back needs.
///
/// Handed to the frontend so an undo is possible without the app holding hidden
/// state: everything needed to reverse the delete is in the value the delete
/// returned.
#[derive(Serialize)]
pub struct TrashedDto {
    /// The file's new location under `Trash/files/`.
    pub trashed: String,
    /// Its `.trashinfo` sidecar, which the restore removes.
    pub info: String,
    /// Where it came from, which is where a restore puts it back.
    pub original: String,
}

/// Move the open file to the freedesktop home trash.
///
/// Trash, never unlink. A viewer's delete key is pressed by someone looking at a
/// picture, often while moving quickly through a folder, and the only acceptable
/// answer to "that was the wrong one" is that it comes back. `restore_file` is the
/// exact inverse, and the delete hands back everything that inverse needs.
///
/// The freedesktop layout and its atomicity live in `arlen-freedesktop-trash`
/// (sidecar first, no-clobber move, canonical paths validated before any side
/// effect) - the same contract `trash-rm` uses, rather than a second trash
/// implementation with its own edge cases.
#[tauri::command]
fn trash_file(path: String) -> Result<TrashedDto, String> {
    // The trash the FILE belongs to, not the one this user's home has. A picture
    // on a USB stick or a second disk cannot be renamed into `$HOME`'s trash - the
    // kernel answers EXDEV - so a home-only delete failed on exactly the media
    // people keep pictures on. `trash_for_current_user` picks the volume's own
    // trash in that case and the home one otherwise.
    let slot = arlen_freedesktop_trash::trash_for_current_user(&path).map_err(|e| {
        use arlen_freedesktop_trash::TrashError as T;
        match e {
            // Still reachable, and now it means something narrower: the home
            // trash could not take it AND the volume has no trash to offer.
            T::CrossDevice => "this file is not on the same drive as your trash, \
                               so it cannot be moved there"
                .to_string(),
            T::NoTrashHere(why) => {
                format!("this drive cannot hold a trash, so the file was not deleted: {why}")
            }
            T::NotFound => "the file is no longer there".to_string(),
            T::Unsupported => "this drive cannot move a file safely enough to undo it".to_string(),
            T::NoSlot => "the trash already holds too many files by that name".to_string(),
            T::NonCanonical => "the trash path could not be resolved".to_string(),
            T::Io(m) => format!("the move failed: {m}"),
        }
    })?;
    let (trashed, trash_info) = slot.into_parts();
    Ok(TrashedDto {
        trashed: trashed.as_str().to_string(),
        info: trash_info.as_str().to_string(),
        original: path,
    })
}

/// Put a trashed file back where it was, and drop its sidecar.
///
/// `rename_noreplace` rather than a plain rename: if something has taken the
/// original name since the delete, the restore refuses rather than overwriting it.
/// The sidecar is removed only after the file is back, so a failed restore leaves
/// the trash entry intact and undoable again.
#[tauri::command]
fn restore_file(trashed: String, info: String, original: String) -> Result<(), String> {
    arlen_freedesktop_trash::rename_noreplace(&trashed, &original)
        .map_err(|e| format!("could not put the file back: {e:?}"))?;
    // A leftover sidecar describes a file that is no longer in the trash; it is
    // untidy rather than harmful, so a failure here does not fail the restore.
    let _ = std::fs::remove_file(&info);
    Ok(())
}

/// The folder a file lives in, its own name, and the file names beside it.
///
/// Non-UTF-8 names are dropped rather than lossily converted: a name that does
/// not round-trip cannot be handed back as a path to open, and counting one
/// would make the total describe files the viewer can never reach.
fn listing(path: &str) -> Result<(PathBuf, String, Vec<String>), String> {
    let here = Path::new(path);
    let dir = here
        .parent()
        .ok_or_else(|| "the file has no parent directory".to_string())?;
    let name = here
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "the file has no readable name".to_string())?;

    let entries: Vec<String> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read the folder: {e}"))?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect();

    Ok((dir.to_path_buf(), name.to_string(), entries))
}

/// Tauri entry point (invoked from `main.rs`).
pub fn run() {
    // This app at info, dependencies at warn, and both halves are a fix.
    //
    // A bare `env_logger::init()` defaults to `error`, so every `log::info!`
    // and `log::warn!` here produced nothing: the app was mute in the journal.
    // That is the failure that made the boot consent hang so hard to find -
    // the component in the middle could not be heard - and it was true of four
    // apps at once.
    //
    // Dependencies stay at warn rather than being swept up to info with it,
    // because zbus logs D-Bus handshake frames WITH their message bytes, and a
    // message body is user content: paths, queries, notification text. At info
    // that lands in a journal no capability grant covers. `RUST_LOG=zbus=trace`
    // still gets it, deliberately.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,arlen_viewers_lib=info"),
    )
    .init();
    // The first non-flag argument is the file to open (`viewer <path>`, the
    // `.desktop` `%f`, or a double-clicked file). Flags (the default-handler
    // subcommands) are consumed earlier in `main`.
    let initial = std::env::args().skip(1).find(|a| !a.starts_with('-'));
    tauri::Builder::default()
        .plugin(tauri_plugin_arlen_shell::init())
        .manage(InitialFile(initial))
        .invoke_handler(tauri::generate_handler![
            decode_image,
            probe_audio,
            detect_media_kind,
            neighbour_file,
            folder_position,
            file_facts,
            trash_file,
            restore_file,
            initial_file
        ])
        .run(tauri::generate_context!())
        .expect("error while running arlen-viewers");
}
