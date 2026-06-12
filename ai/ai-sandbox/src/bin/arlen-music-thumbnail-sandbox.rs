//! The sandboxed music cover-art thumbnail worker.
//!
//! Locks itself down with [`arlen_ai_sandbox::apply_sandbox`] (no new
//! privileges, no filesystem, no network), then reads untrusted audio bytes from
//! stdin, extracts the embedded cover art, decodes and downscales it to a
//! thumbnail, and writes the PNG to stdout. Both the media-container parse (the
//! tag reader) and the image decode are memory-unsafe attack surfaces, so they
//! only ever happen here, behind the lockdown; the parent
//! ([`arlen_ai_sandbox::album_art_thumbnail`]) feeds it the untrusted file and
//! reads back only the re-encoded thumbnail.
//!
//! A file with no usable embedded art exits 0 with EMPTY stdout - the parent
//! reads that as "no art, fall back to the music icon", not a failure. A non-zero
//! exit means the sandbox or the parse failed; the parent treats it fail-closed
//! (also the icon). This binary is built only with the `music` feature.

#[cfg(target_os = "linux")]
fn main() {
    use std::io::{Read, Write};

    // Close every inherited descriptor beyond stdio before anything else.
    // Landlock does not revoke already-open fds, so a leaked parent handle would
    // otherwise survive into the worker and remain usable by exploited parser
    // code. (Mirrors arlen-thumbnail-sandbox; kept per-worker as the local
    // lockdown step.)
    close_inherited_fds();

    // Lock down before touching any untrusted input. If the sandbox cannot be
    // installed we refuse to parse rather than run exposed.
    if let Err(e) = arlen_ai_sandbox::apply_sandbox() {
        eprintln!("sandbox setup failed: {e}");
        std::process::exit(3);
    }

    let mut input = Vec::new();
    if let Err(e) = std::io::stdin()
        .take((arlen_ai_sandbox::MAX_BYTES as u64) + 1)
        .read_to_end(&mut input)
    {
        eprintln!("read stdin failed: {e}");
        std::process::exit(4);
    }

    // Extract the embedded picture. No art -> exit 0 with no output (the parent's
    // fall-back-to-icon signal); a container parse failure is fail-closed.
    let art = match arlen_ai_sandbox::extract_album_art(&input) {
        Ok(Some(art)) => art,
        Ok(None) => return,
        Err(e) => {
            eprintln!("album-art extract failed: {e}");
            std::process::exit(5);
        }
    };

    // Decode + downscale the extracted picture. Art that will not decode is also
    // no-thumbnail: exit 0 with no output, so the tile shows the music icon
    // rather than the listing failing on one malformed embed.
    match arlen_ai_sandbox::generate_thumbnail(&art, arlen_ai_sandbox::THUMBNAIL_MAX_DIM) {
        Ok(thumb) => {
            if std::io::stdout().write_all(&thumb).is_err() {
                std::process::exit(6);
            }
        }
        Err(e) => {
            eprintln!("thumbnail of embedded art failed: {e}");
        }
    }
}

/// Close every inherited file descriptor above stderr, fail-closed.
///
/// Prefers `close_range`; if that is unavailable (very old kernel) or fails,
/// falls back to enumerating `/proc/self/fd` and closing each descriptor above
/// stderr. A numeric ceiling is not a reliable upper bound, so we enumerate the
/// exact open set. Runs before the sandbox, while `/proc` is still readable.
#[cfg(target_os = "linux")]
fn close_inherited_fds() {
    // SAFETY: close_range only closes descriptors in the range; it takes no
    // pointers and cannot corrupt memory.
    let rc = unsafe { libc::close_range(3, libc::c_uint::MAX, 0) };
    if rc == 0 {
        return;
    }
    let rd = match std::fs::read_dir("/proc/self/fd") {
        Ok(rd) => rd,
        Err(e) => {
            // Cannot guarantee inherited fds are gone: fail closed.
            eprintln!("cannot enumerate file descriptors to close: {e}");
            std::process::exit(8);
        }
    };
    // Collect first, then close, so closing the directory's own fd mid-iteration
    // cannot truncate the listing. A mid-stream error or an unparsable name means
    // we cannot trust the listing, so fail closed.
    let mut fds: Vec<i32> = Vec::new();
    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("file descriptor enumeration failed mid-stream: {e}");
                std::process::exit(8);
            }
        };
        let name = entry.file_name();
        let Some(num) = name.to_str().and_then(|s| s.parse::<i32>().ok()) else {
            eprintln!("unparsable file-descriptor entry: {name:?}");
            std::process::exit(8);
        };
        if num > 2 {
            fds.push(num);
        }
    }
    for fd in fds {
        // SAFETY: close on an int fd; closing an already-closed fd returns
        // EBADF, which is harmless.
        unsafe {
            libc::close(fd);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("arlen-music-thumbnail-sandbox requires Linux (Landlock + seccomp)");
    std::process::exit(2);
}
