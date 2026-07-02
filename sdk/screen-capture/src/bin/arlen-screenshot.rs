//! `arlen-screenshot`: the first-party screenshot tool (screenshot-capture-plan.md).
//!
//! Being built incrementally, each slice runtime-verified under a nested compositor.
//! Slice 1: probe the compositor for capture support and report it, so the capture
//! path can be verified end-to-end (`dev/screenshot/shoot-compositor.sh` runs this
//! as the Wayland client under cosmic-comp nested). The capture modes + targets land
//! in later slices.

use anyhow::{anyhow, Context, Result};
use arlen_screen_capture::{
    capture_output, capture_region, capture_support, capture_window, list_outputs, list_windows,
    write_png, CapturedImage, COPY_MANAGER_INTERFACE, OUTPUT_SOURCE_MANAGER_INTERFACE,
    TOPLEVEL_SOURCE_MANAGER_INTERFACE,
};

fn main() -> Result<()> {
    // Usage:
    //   arlen-screenshot                 probe capture support
    //   arlen-screenshot --list          list the capturable outputs
    //   arlen-screenshot <file>          capture output 0 to a PNG
    //   arlen-screenshot -g X,Y,W,H <file>   capture a region of output 0
    let raw: Vec<String> = std::env::args().skip(1).collect();
    // `-c` / `--cursor` (anywhere) paints the pointer onto the capture.
    let cursor = raw.iter().any(|a| a == "-c" || a == "--cursor");
    let args: Vec<String> = raw
        .into_iter()
        .filter(|a| a != "-c" && a != "--cursor")
        .collect();
    match args.first().map(String::as_str) {
        Some("--list") => {
            for o in list_outputs()? {
                println!(
                    "output {}: {} {}x{}",
                    o.index,
                    o.name.as_deref().unwrap_or("?"),
                    o.width,
                    o.height
                );
            }
            return Ok(());
        }
        Some("--list-windows") => {
            for w in list_windows()? {
                println!(
                    "window {}: [{}] {}",
                    w.index,
                    w.app_id.as_deref().unwrap_or("?"),
                    w.title.as_deref().unwrap_or("?")
                );
            }
            return Ok(());
        }
        Some("--shot") => {
            let image = capture_output(0, cursor)?;
            let path = match args.get(1) {
                Some(p) => p.clone(),
                None => default_out()?,
            };
            save(&image, &path)?;
            return Ok(());
        }
        Some("-g") => {
            let geom = args.get(1).ok_or_else(|| anyhow!("-g needs a X,Y,W,H region"))?;
            let (x, y, w, h) = parse_region(geom)?;
            let image = capture_region(0, x, y, w, h, cursor)?;
            let path = match args.get(2) {
                Some(p) => p.clone(),
                None => default_out()?,
            };
            save(&image, &path)?;
            return Ok(());
        }
        Some("--window") => {
            let index: usize = args
                .get(1)
                .ok_or_else(|| anyhow!("--window needs an index (see --list-windows)"))?
                .parse()
                .map_err(|e| anyhow!("bad window index: {e}"))?;
            let image = capture_window(index, cursor)?;
            let path = match args.get(2) {
                Some(p) => p.clone(),
                None => default_out()?,
            };
            save(&image, &path)?;
            return Ok(());
        }
        Some(path) => {
            let image = capture_output(0, cursor)?;
            save(&image, path)?;
            return Ok(());
        }
        None => {}
    }

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

/// Write a capture to `path` and report it.
fn save(image: &CapturedImage, path: &str) -> Result<()> {
    write_png(image, std::path::Path::new(path))?;
    println!("captured {}x{} to {}", image.width, image.height, path);
    Ok(())
}

/// The default save path: a timestamped file in the screenshots directory, whose
/// parent is created if missing (`Screenshot-%Y%m%d-%H%M%S.png`).
fn default_out() -> Result<String> {
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let dir = arlen_screen_capture::screenshots_dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    Ok(dir
        .join(arlen_screen_capture::default_filename(&timestamp))
        .to_string_lossy()
        .into_owned())
}

/// Parse a `X,Y,W,H` region string into pixel bounds.
fn parse_region(s: &str) -> Result<(u32, u32, u32, u32)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        return Err(anyhow!("region must be X,Y,W,H, got {s:?}"));
    }
    let field = |i: usize| -> Result<u32> {
        parts[i]
            .trim()
            .parse::<u32>()
            .map_err(|e| anyhow!("bad region field {:?}: {e}", parts[i]))
    };
    Ok((field(0)?, field(1)?, field(2)?, field(3)?))
}
