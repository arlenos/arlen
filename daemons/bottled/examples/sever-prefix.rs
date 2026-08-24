//! Cut one Wine prefix loose from the home `wineboot` wired it into, and say what
//! it did.
//!
//! The bottle daemon calls the library directly; this exists so the same pass can
//! be run against a prefix by hand, which is how the severing was checked against
//! a real wine-11.14 prefix rather than against a fixture that agrees with me.
//!
//! Usage: `cargo run --example sever-prefix -- <prefix> [--dry-run]`

use std::path::PathBuf;

use arlen_wine_core::sever::{apply, plan, prefix_links, still_escaping, Sever};

fn main() -> std::io::Result<()> {
    let mut args = std::env::args().skip(1);
    let prefix = match args.next() {
        Some(p) => PathBuf::from(p),
        None => {
            eprintln!("usage: sever-prefix <prefix> [--dry-run]");
            std::process::exit(2);
        }
    };
    let dry_run = args.any(|a| a == "--dry-run");

    let links = prefix_links(&prefix)?;
    if links.is_empty() {
        eprintln!(
            "{} holds no links, so it has not been booted yet and there is nothing to cut",
            prefix.display()
        );
        std::process::exit(1);
    }
    // Nothing is granted at severing time: the drive table is written after.
    let steps = plan(&prefix, &links, &[]);
    for step in &steps {
        match step {
            Sever::Remove(p) => println!("remove  {}", p.display()),
            Sever::Replace(p) => println!("replace {}", p.display()),
        }
    }
    if dry_run {
        return Ok(());
    }
    let done = apply(&steps)?;
    println!("{} link(s) cut", done.len());

    // Nothing is granted yet at severing time: the drive table is written after.
    let left = still_escaping(&prefix, &[])?;
    if left.is_empty() {
        println!("nothing in the prefix reaches out of it any more");
    } else {
        for l in &left {
            eprintln!("still leaves the prefix: {}", l.display());
        }
        std::process::exit(1);
    }
    Ok(())
}
