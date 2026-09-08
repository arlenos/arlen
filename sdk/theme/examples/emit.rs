//! Write the foreign-toolkit files for a bundled theme into a directory, so a
//! GTK or Qt window can be started against them and looked at
//! (`dev/screenshot/shoot-toolkits.sh`). The mapping in `gtk.rs` and `qt.rs` is
//! a design decision, and a design decision is checked with a picture.
//!
//! Usage: `cargo run --example emit -- <dark|light> <config-dir>`

use std::path::Path;

use arlen_theme::{
    apply::write_foreign_toolkit_configs, wine::generate_wine_reg, ArlenTheme, DARK_TOML, LIGHT_TOML,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let variant = args.next().unwrap_or_else(|| "dark".to_string());
    let dir = args.next().unwrap_or_else(|| usage());
    let bundled = match variant.as_str() {
        "dark" => DARK_TOML,
        "light" => LIGHT_TOML,
        _ => usage(),
    };
    let theme = ArlenTheme::from_bundled(bundled).expect("the bundled theme resolves");
    let report = write_foreign_toolkit_configs(&theme, Path::new(&dir));
    // Wine is per bottle and `apply` leaves it to `bottled`; here one document
    // at 96 DPI is enough for a throwaway prefix to import and be looked at.
    let wine = Path::new(&dir).join("wine.reg");
    match std::fs::write(&wine, generate_wine_reg(&theme, 1.0)) {
        Ok(()) => println!("wrote {}", wine.display()),
        Err(e) => eprintln!("failed {}: {e}", wine.display()),
    }
    for p in &report.written {
        println!("wrote {}", p.display());
    }
    // The skipped ones matter as much as the written ones and this printed
    // neither, so a run against a real home said "wrote 17 files" while quietly
    // leaving six of the person's own alone. Both readings of that silence are
    // wrong: it looks like the theme reached everything, and when it has NOT
    // reached a toolkit there is nothing on screen saying why.
    for p in &report.skipped_foreign {
        println!("kept {} (not ours, so not overwritten)", p.display());
    }
    for (p, why) in &report.errors {
        eprintln!("failed {}: {why}", p.display());
    }
    if !report.is_clean() {
        std::process::exit(1);
    }
}

fn usage() -> ! {
    eprintln!("usage: emit <dark|light> <config-dir>");
    std::process::exit(2);
}
