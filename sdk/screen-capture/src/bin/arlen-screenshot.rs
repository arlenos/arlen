//! `arlen-screenshot`: the first-party screenshot tool (screenshot-capture-plan.md).
//!
//! Being built incrementally, each slice runtime-verified under a nested compositor.
//! Slice 1: probe the compositor for capture support and report it, so the capture
//! path can be verified end-to-end (`dev/screenshot/shoot-compositor.sh` runs this
//! as the Wayland client under cosmic-comp nested). The capture modes + targets land
//! in later slices.

use anyhow::Result;
use arlen_screen_capture::{
    capture_support, COPY_MANAGER_INTERFACE, OUTPUT_SOURCE_MANAGER_INTERFACE,
    TOPLEVEL_SOURCE_MANAGER_INTERFACE,
};

fn main() -> Result<()> {
    let support = capture_support()?;

    println!("advertised globals ({}):", support.globals.len());
    for g in &support.globals {
        println!("  {} v{}", g.interface, g.version);
    }

    println!("capture support:");
    report("frame copy", COPY_MANAGER_INTERFACE, support.has_copy_manager());
    report(
        "output source",
        OUTPUT_SOURCE_MANAGER_INTERFACE,
        support.has_output_source_manager(),
    );
    report(
        "window source",
        TOPLEVEL_SOURCE_MANAGER_INTERFACE,
        support.has_toplevel_source_manager(),
    );

    // The frame-copy manager is load-bearing: without it there is nothing to build
    // on, so a compositor that lacks it is a hard failure the caller should see.
    if !support.has_copy_manager() {
        anyhow::bail!(
            "the compositor does not advertise {COPY_MANAGER_INTERFACE}; \
             ext-image-copy-capture capture is unavailable"
        );
    }

    // Probe a capture session for the first output and report the buffer
    // constraints the compositor wants (the handshake before a real capture).
    let constraints = arlen_screen_capture::probe_session(0)?;
    println!(
        "output 0 capture: {}x{}, shm formats {:?}",
        constraints.width, constraints.height, constraints.shm_formats
    );
    Ok(())
}

fn report(label: &str, interface: &str, present: bool) {
    let mark = if present { "yes" } else { "NO" };
    println!("  {label:<14} [{mark}] {interface}");
}
