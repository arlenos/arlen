//! Launch a program in a saved bottle, reading the bottle from disk.
//!
//! The last piece of the round trip: everything the launch needs comes out of
//! `bottle.toml`, so what runs is what was recorded, not what a caller happened to
//! pass. The bottle daemon will do this; the example is how it was checked against
//! real Wine.
//!
//! Usage: `cargo run --example run-in-bottle -- <bottles-dir> <id> <program> [args]`

use std::collections::BTreeMap;
use std::path::Path;

use arlen_wine_core::bottle::{bottle_run, unmet_drives};
use arlen_wine_core::plumbing::plumbing_binds;
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
    let mut env = BTreeMap::new();
    env.insert("WINEPREFIX".into(), bottle.prefix_root.display().to_string());
    // The prefix is the bottle's home. Wine writes a cache beside it and nothing
    // of the person's is reachable to write into anyway.
    env.insert("HOME".into(), bottle.prefix_root.display().to_string());
    env.insert("PATH".into(), "/usr/bin".into());
    env.insert("WINEDEBUG".into(), "-all".into());
    env.insert("WINEDLLOVERRIDES".into(), "mscoree,mshtml=".into());
    if let Ok(display) = std::env::var("DISPLAY") {
        env.insert("DISPLAY".into(), display);
    }

    let binds = plumbing_binds(&bottle.plumbing, Path::new(&runtime_dir), |p| p.exists());
    let run = match bottle_run(&bottle, Path::new("/usr"), env, binds) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    // Refuse rather than launch a bottle whose drive letters promise more than the
    // sandbox delivers: the program would meet a drive it cannot open and no error
    // anyone could act on.
    let unmet = unmet_drives(&run.confinement, &run.drives);
    if !unmet.is_empty() {
        for u in &unmet {
            eprintln!("{}: promises {:?}, sandbox gives {:?}", u.letter, u.promised, u.actual);
        }
        std::process::exit(1);
    }

    let mut argv = run.confinement.bwrap_args();
    for root in arlen_confiner::merged_usr_compat_roots() {
        argv.push("--ro-bind".into());
        argv.push(root.clone());
        argv.push(root);
    }
    argv.push("--".into());
    argv.push("/usr/bin/wine".into());
    argv.extend(a[3..].iter().cloned());

    let status = std::process::Command::new("bwrap").args(&argv).status();
    match status {
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("bwrap could not start: {e}");
            std::process::exit(1);
        }
    }
}
