//! The decoder worker's self-applied Landlock confinement (`quickview-plan.md`).
//!
//! Each worker calls [`apply_decoder_landlock`] as the very first thing in
//! `main()`, before it reads a single byte of the untrusted file. By then the
//! dynamic loader has already mapped the worker's libraries (that happens at
//! exec, before `main`), and stdin/stdout are inherited open fds (Landlock does
//! not gate reads/writes on already-open fds), so the worker needs no new file
//! access to do its job. The ruleset therefore grants **read + execute** only
//! under `/usr` and the merged-usr `/lib*` (for any lazy locale load or a
//! codec-plugin `dlopen`, e.g. libheif's backends) and **no write anywhere**.
//!
//! This is the third confinement layer, under the bwrap mount namespace (which
//! already makes `/usr` read-only and binds no writable input) and the
//! per-decoder seccomp filter. Its added value against a decoder RCE: the mount
//! view still leaves `/tmp` (tmpfs) and `/dev` writable and `/proc` readable;
//! Landlock removes those too, so a compromised decoder cannot stage a payload
//! on disk or read `/proc`/`/dev` paths. It only ever *reduces* access, so it
//! composes safely with the outer layers.
//!
//! Fail-closed: if the kernel does not enforce the ruleset, the worker must not
//! proceed believing it is confined when it is not.

#![cfg(target_os = "linux")]

use std::io;

use landlock::{
    Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus,
    ABI,
};

fn ll_err(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::other(e)
}

/// Lock the current (decoder worker) process to read-only `/usr` + `/lib*` and
/// deny all filesystem writes. Call once at the top of `main()`, before reading
/// the input. Returns `Err` on a ruleset failure or a kernel that did not
/// enforce it (the worker then exits without decoding).
pub fn apply_decoder_landlock() -> io::Result<()> {
    let abi = ABI::V5;
    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .map_err(ll_err)?
        .create()
        .map_err(ll_err)?;

    // Read + execute under the system library trees: read for a lazy locale or
    // config load, execute for a codec-plugin dlopen (libheif loads its dav1d /
    // libde265 backends this way). The mount namespace already made these
    // read-only, so this grants no write. Everything else (`/tmp`, `/dev`,
    // `/proc`, `$HOME`) is left with no rule, hence no access at all.
    let read_exec = AccessFs::from_read(abi) | AccessFs::Execute;
    for dir in ["/usr", "/lib64", "/lib"] {
        // A path that cannot be opened is skipped (fail-safe: less access, never
        // more); on a merged-usr host /lib64 and /lib resolve under /usr anyway.
        if let Ok(fd) = PathFd::new(dir) {
            ruleset = ruleset
                .add_rule(PathBeneath::new(fd, read_exec))
                .map_err(ll_err)?;
        }
    }

    let status = ruleset.restrict_self().map_err(ll_err)?;
    if status.ruleset == RulesetStatus::NotEnforced {
        return Err(io::Error::other("landlock ruleset not enforced by the kernel"));
    }
    Ok(())
}
