//! I18N-R3 born-RTL gate: fail on NEW physical directional CSS, accept the
//! existing via a baseline (the same honest baseline-diff the born-translatable
//! lint uses - retrofitting the baselined usages to logical properties is the
//! later migration, so the gate only ever cares about usages NOT in the baseline).
//!
//! Usage:
//!   arlen-rtl-lint [--root <dir>]... [--baseline <file>] [--update]
//!     --root      a directory tree to scan (repeatable; default `apps`)
//!     --baseline  the accepted-usages file (default `dev/rtl-baseline.tsv`)
//!     --update    rewrite the baseline from the current findings (then exit 0)
//! Exit code 0 = no new physical CSS (or `--update`); 1 = new usages; 2 = usage/IO.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use arlen_i18n::rtl::scan_rtl;

/// Recursively collect `.svelte` + `.css` files under `root`, skipping vendored
/// and build trees. Sorted for a deterministic baseline.
fn collect(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if matches!(
                name.as_ref(),
                "node_modules" | "build" | ".svelte-kit" | "target" | ".git" | "dist"
            ) {
                continue;
            }
            collect(&path, out);
        } else if name.ends_with(".svelte") || name.ends_with(".css") {
            out.push(path);
        }
    }
}

/// The baseline key: `relative/path\tfound`. The line is excluded so a usage that
/// merely moves within a file is not seen as new.
fn key(rel: &str, found: &str) -> String {
    format!("{rel}\t{found}")
}

struct Args {
    roots: Vec<PathBuf>,
    baseline: PathBuf,
    update: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut roots = Vec::new();
    let mut baseline = PathBuf::from("dev/rtl-baseline.tsv");
    let mut update = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--root" => roots.push(PathBuf::from(it.next().ok_or("--root needs a value")?)),
            "--baseline" => baseline = PathBuf::from(it.next().ok_or("--baseline needs a value")?),
            "--update" => update = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::from("apps"));
    }
    Ok(Args { roots, baseline, update })
}

fn load_baseline(path: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(str::to_string).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("arlen-rtl-lint: {e}");
            return ExitCode::from(2);
        }
    };

    let mut files = Vec::new();
    for root in &args.roots {
        collect(root, &mut files);
    }
    files.sort();

    let mut current: BTreeSet<String> = BTreeSet::new();
    let mut report: Vec<(String, usize, String, String)> = Vec::new();
    for path in &files {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("arlen-rtl-lint: cannot read {}: {e}", path.display());
                return ExitCode::from(2);
            }
        };
        let rel = path.to_string_lossy().replace('\\', "/");
        for f in scan_rtl(&src) {
            let k = key(&rel, &f.found);
            if current.insert(k) {
                report.push((rel.clone(), f.line, f.found, f.suggestion));
            }
        }
    }

    if args.update {
        let body: String = current.iter().map(|k| format!("{k}\n")).collect();
        if let Some(parent) = args.baseline.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&args.baseline, body) {
            eprintln!("arlen-rtl-lint: cannot write baseline {}: {e}", args.baseline.display());
            return ExitCode::from(2);
        }
        println!(
            "arlen-rtl-lint: baseline updated with {} usages -> {}",
            current.len(),
            args.baseline.display()
        );
        return ExitCode::SUCCESS;
    }

    let baseline = load_baseline(&args.baseline);
    let mut new_usages: Vec<&(String, usize, String, String)> = report
        .iter()
        .filter(|(rel, _, found, _)| !baseline.contains(&key(rel, found)))
        .collect();
    new_usages.sort_by(|a, b| (a.0.as_str(), a.1).cmp(&(b.0.as_str(), b.1)));

    if new_usages.is_empty() {
        println!("arlen-rtl-lint: no new physical directional CSS");
        return ExitCode::SUCCESS;
    }
    eprintln!(
        "arlen-rtl-lint: {} new physical directional usage(s) - use the logical form so the layout mirrors under dir=rtl:",
        new_usages.len()
    );
    for (rel, line, found, suggestion) in new_usages {
        eprintln!("  {rel}:{line}  {found}  ->  {suggestion}");
    }
    ExitCode::from(1)
}
