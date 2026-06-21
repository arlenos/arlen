//! The sandboxed AVIF/HEIC codec worker (`apps/viewers`, quickview-plan.md).
//!
//! Identical containment to `arlen-image-view-sandbox` EXCEPT it installs the
//! THREADED seccomp profile ([`arlen_ai_sandbox::apply_sandbox_threaded`]): the
//! C-linked decoders (AVIF via dav1d) spawn a decode thread pool, so this one
//! worker permits `clone`. The pure-Rust workers keep the tight profile and
//! never run a C decoder - the loosening is confined here (per-decoder
//! isolation, the approved AVIF/HEIC posture). It reads untrusted image bytes,
//! decodes at full resolution and writes the sanitised PNG; a non-zero exit
//! means no image, fail-closed. Built only with the `codec` feature (which links
//! dav1d).

#[cfg(target_os = "linux")]
fn main() {
    use std::io::{Read, Write};

    // Close inherited fds before lockdown (Landlock does not revoke open fds).
    close_inherited_fds();

    // The THREADED profile - the one place clone is permitted, for the decode
    // thread pool. If it cannot be installed we refuse rather than run exposed.
    if let Err(e) = arlen_ai_sandbox::apply_sandbox_threaded() {
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

    // `decode_view_image` routes through image-rs, which with the `codec`
    // feature's `avif-native` decodes AVIF (and other ftyp formats), re-encoding
    // a sanitised full-resolution PNG.
    match arlen_ai_sandbox::decode_view_image(&input) {
        Ok(png) => {
            if std::io::stdout().write_all(&png).is_err() {
                std::process::exit(6);
            }
        }
        Err(e) => {
            eprintln!("codec decode failed: {e}");
            std::process::exit(5);
        }
    }
}

/// Close every inherited file descriptor above stderr, fail-closed. Prefers
/// `close_range`, falling back to enumerating `/proc/self/fd`. Runs before the
/// sandbox, while `/proc` is still readable. (Mirrors the sibling workers.)
#[cfg(target_os = "linux")]
fn close_inherited_fds() {
    // SAFETY: close_range closes only descriptors in the range; no pointers.
    let rc = unsafe { libc::close_range(3, libc::c_uint::MAX, 0) };
    if rc == 0 {
        return;
    }
    let rd = match std::fs::read_dir("/proc/self/fd") {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("cannot enumerate file descriptors to close: {e}");
            std::process::exit(8);
        }
    };
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
        // SAFETY: close on an int fd; EBADF on an already-closed fd is harmless.
        unsafe {
            libc::close(fd);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("arlen-image-codec-sandbox requires Linux (Landlock + seccomp)");
    std::process::exit(2);
}
