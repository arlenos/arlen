//! Arlen kernel-layer daemon.
//!
//! Loads the eBPF program into the kernel, reads FileOpenedEvents from the
//! ring buffer, and forwards them to the Arlen Event Bus.
//!
//! Must run as root (or with CAP_BPF + CAP_PERFMON).

use anyhow::{Context, Result};
use aya::{
    Ebpf,
    maps::RingBuf,
    programs::TracePoint,
};
use aya_log::EbpfLogger;
use log::{info, warn};
use tokio::signal;

mod normalizer;

/// Resolve a daemon socket path per the standard Arlen 3-tier
/// convention: the `env_var` override (non-empty) wins, else
/// `$XDG_RUNTIME_DIR/arlen/<file_name>` (the per-user path, i.e.
/// `/run/user/{uid}/arlen/<file_name>`), else `/run/arlen/<file_name>`.
///
/// kernel-layer does not depend on `os-sdk`, so the shared
/// `os_sdk::runtime::socket_path` resolver is reproduced here. The
/// `ARLEN_PRODUCER_SOCKET` override stays tier 1 — the dev stack and
/// the integration harness pin the bus socket through it. NB this
/// daemon usually runs as root outside a user session, so the
/// `/run/arlen` last resort is the common path; a per-user launcher
/// that pins the env still wins.
fn socket_path(env_var: &str, file_name: &str) -> String {
    let env_val = std::env::var(env_var).ok();
    if let Some(p) = env_val.as_deref().filter(|s| !s.is_empty()) {
        return p.to_string();
    }
    if let Some(dir) = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
    {
        let path = format!("{dir}/arlen/{file_name}");
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        return path;
    }
    format!("/run/arlen/{file_name}")
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let producer_socket = socket_path("ARLEN_PRODUCER_SOCKET", "event-bus-producer.sock");

    // Read or generate session ID.
    // Read, never mint - `arlen-session` is the one thing that knows how many
    // sessions a login has. This used to invent its own uuid, and the compositor
    // invented a different one, so the same login carried two ids and a file the
    // user opened could not be joined to the window they had focused.
    //
    // Absent is a deployment defect and is said so: substituting would make the
    // graph look joined while joining nothing.
    // A SYSTEM SOURCE, not a session. This looked for `ARLEN_SESSION_ID` and
    // logged an error when it was missing, which it always is: the session
    // exports that variable to its own children, and this sensor is a system
    // unit started at boot, before any session exists. It cannot have one.
    //
    // The bus already accounts for this - `origin` takes "a session reference, or
    // a named system source for the things that happen in no session", and its
    // rule refuses only an EMPTY origin. So the sensor names itself rather than
    // sending a blank the bus would reject or borrowing a session id that would
    // attribute machine-wide events to whichever session happened to start first.
    //
    // A session-scoped consumer can still find its own events: they carry the
    // uid, which is what the bus filters on.
    let session_id = std::env::var("ARLEN_SESSION_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "system:kernel-layer".to_string());

    info!("starting kernel-layer daemon");
    info!("origin={session_id}");

    // The path comes from build.rs, which locates the object under whichever
    // target directory cargo is using. A literal relative path assumed the target
    // dir sat beside this crate and broke as soon as the repo set a shared one.
    let ebpf_owned = Box::leak(Box::new(Ebpf::load(aya::include_bytes_aligned!(
        env!("ARLEN_EBPF_OBJECT")
    ))
    .context("failed to load eBPF program")?));

    let ebpf: &'static mut _ = ebpf_owned;

    if let Err(e) = EbpfLogger::init(ebpf) {
        warn!("eBPF logger init failed (non-fatal): {e}");
    }

    // Load and attach all programs first (before taking map references).
    {
        let prog: &mut TracePoint = ebpf
            .program_mut("file_opened")
            .context("program 'file_opened' not found")?
            .try_into()?;
        prog.load()?;
        prog.attach("syscalls", "sys_enter_openat")
            .context("failed to attach to sys_enter_openat")?;
        info!("eBPF tracepoint attached to sys_enter_openat");
    }
    {
        let prog: &mut TracePoint = ebpf
            .program_mut("process_exec")
            .context("program 'process_exec' not found")?
            .try_into()?;
        prog.load()?;
        prog.attach("sched", "sched_process_exec")
            .context("failed to attach to sched_process_exec")?;
        info!("eBPF tracepoint attached to sched_process_exec");
    }
    {
        let prog: &mut TracePoint = ebpf
            .program_mut("file_written")
            .context("program 'file_written' not found")?
            .try_into()?;
        prog.load()?;
        prog.attach("syscalls", "sys_enter_write")
            .context("failed to attach to sys_enter_write")?;
        info!("eBPF tracepoint attached to sys_enter_write");
    }
    {
        let prog: &mut TracePoint = ebpf
            .program_mut("net_state_change")
            .context("program 'net_state_change' not found")?
            .try_into()?;
        prog.load()?;
        prog.attach("sock", "inet_sock_set_state")
            .context("failed to attach to inet_sock_set_state")?;
        info!("eBPF tracepoint attached to inet_sock_set_state");
    }

    // Take ownership of maps (avoids multiple mutable borrows of ebpf).
    let ring_buf = RingBuf::try_from(ebpf.take_map("EVENTS").context("EVENTS map not found")?)?;
    let ring_buf_exec = RingBuf::try_from(ebpf.take_map("EXEC_EVENTS").context("EXEC_EVENTS map not found")?)?;
    let ring_buf_write = RingBuf::try_from(ebpf.take_map("WRITE_EVENTS").context("WRITE_EVENTS map not found")?)?;
    let ring_buf_net = RingBuf::try_from(ebpf.take_map("NET_EVENTS").context("NET_EVENTS map not found")?)?;

    let producer_socket_clone = producer_socket.clone();
    let session_id_clone = session_id.clone();
    tokio::task::spawn_blocking(move || {
        normalizer::run(
            ring_buf,
            ring_buf_exec,
            ring_buf_write,
            ring_buf_net,
            &producer_socket_clone,
            &session_id_clone,
        )
    });

    signal::ctrl_c().await?;
    info!("shutting down");
    Ok(())
}
#[cfg(test)]
mod normalizer_tests;
