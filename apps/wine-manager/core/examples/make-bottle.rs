//! Make a bottle end to end, booting the prefix with the real `wineboot`.
//!
//! The bottle daemon will do this; the example exists so the sequence can be run
//! by hand against real Wine, which is how it was checked.
//!
//! Usage: `cargo run --example make-bottle -- <bottles-dir> <id> <granted-dir>`

use std::path::{Path, PathBuf};

use arlen_wine_core::bottle::Egress;
use arlen_wine_core::create::{create_bottle, NewBottle};
use arlen_wine_core::plumbing::{Display, Plumbing};
use arlen_wine_core::{Access, PathGrant};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 4 {
        eprintln!("usage: make-bottle <bottles-dir> <id> <granted-dir>");
        std::process::exit(2);
    }
    let new = NewBottle {
        id: a[2].clone(),
        grants: vec![PathGrant {
            host: PathBuf::from(&a[3]),
            access: Access::ReadWrite,
        }],
        egress: Egress::None,
        plumbing: Plumbing {
            display: Display::X11,
            gpu: false,
            fonts: true,
        },
    };
    let boot = |prefix: &Path| -> Result<(), String> {
        // Deliberately `status()` with null stdio rather than `output()`.
        // `wineboot` starts a `wineserver` that outlives it and inherits whatever
        // stdout and stderr it was given, so a pipe can outlive the process that
        // was being waited for. That is the documented shape of wine's process
        // model; what I measured here is only that the first attempt with
        // `output()` never returned, and later hangs had a second cause (orphaned
        // wine clients whose server had been killed), so treat the pipe as a
        // reason to avoid `output()` rather than as the proven single cause.
        let status = std::process::Command::new("wineboot")
            .arg("-u")
            .env("WINEPREFIX", prefix)
            .env("WINEDEBUG", "-all")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("wineboot exited with {status}"));
        }
        // Stop the server this started. A bottle that is being made is not a bottle
        // that is running, and leaving the process behind means the next launch
        // inherits a server nobody meant to keep.
        let _ = std::process::Command::new("wineserver")
            .arg("-k")
            .env("WINEPREFIX", prefix)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        Ok(())
    };
    match create_bottle(Path::new(&a[1]), &new, boot) {
        Ok(b) => println!("made {} at {}", b.id, b.prefix_root.display()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
