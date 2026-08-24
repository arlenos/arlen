//! Launch a program in a saved bottle, reading the bottle from disk.
//!
//! Everything the launch needs comes out of `bottle.toml`, so what runs is what
//! was recorded rather than what a caller happened to pass. The argv assembly and
//! its refusals live in `arlen_wine_core::launch`; this is the spawn around them,
//! and the way the whole path was checked against real Wine.
//!
//! Usage: `cargo run --example run-in-bottle -- <bottles-dir> <id> <program> [args]`

use std::path::Path;

use arlen_wine_core::launch::launch_argv;
use arlen_wine_core::registry::load_bottle;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        eprintln!("usage: run-in-bottle <bottles-dir> <id> <program> [args]");
        std::process::exit(2);
    }
    let bottle = match load_bottle(Path::new(&a[1]), &a[2]) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/run/user/1000".into());
    let display = std::env::var("DISPLAY").ok();
    let argv = match launch_argv(
        &bottle,
        Path::new("/usr"),
        Path::new(&runtime_dir),
        display.as_deref(),
        &a[3..],
        |p| p.exists(),
    ) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    match std::process::Command::new("bwrap").args(&argv).status() {
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("bwrap could not start: {e}");
            std::process::exit(1);
        }
    }
}
