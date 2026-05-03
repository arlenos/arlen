//! Manual test harness for the `shell.search.open` SDK + IPC
//! broker.
//!
//! Build:
//!   cargo build -p os-sdk --example search_test
//!
//! Run with the desktop-shell already up. The binary lives at
//! `target/debug/examples/search_test` so identity resolution maps
//! it to `dev.search_test` (per `sdk/permissions::identity` debug
//! fallback). Drop a profile at
//! `~/.config/permissions/dev.search_test.toml`:
//!
//!   [info]
//!   app_id = "dev.search_test"
//!   tier = "first-party"
//!
//!   [search]
//!   open = true
//!
//! Without that file, the broker returns PermissionDenied for every
//! request (foundation §7.3 explicit-grant default-deny).
//!
//! Subcommands:
//!   <query>                                  open with query
//!   --mode=files <query>                     open with files mode
//!   --mode=ai <query>                        open with ai mode
//!   --mode=apps <query>                      open with apps mode
//!   --no-args                                empty query (super-key
//!                                            equivalent)
//!
//! What this exercises:
//! - SO_PEERCRED + identity resolution (broker logs
//!   "connection from app_id=dev.search_test pid=...")
//! - Per-request `[search] open` scope check
//! - Audit log emission (visible in shell log; verify NO query
//!   content appears in audit lines — that's the leak guard)
//! - Mode-prefix injection (mode=files, query=foo → launcher
//!   prefilled with "files: foo")

use std::env;

use os_sdk::search::{SearchMode, UnixSearchClient};

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let (mode, query) = parse_args(&args);

    let client = match UnixSearchClient::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("client construct failed: {e}");
            std::process::exit(2);
        }
    };

    match client.open(&query, mode).await {
        Ok(()) => {
            println!("open: OK (query={:?} mode={:?})", query, mode);
        }
        Err(e) => {
            eprintln!("open FAILED: {e}");
            std::process::exit(1);
        }
    }
}

fn parse_args(args: &[String]) -> (Option<SearchMode>, String) {
    let mut mode: Option<SearchMode> = None;
    let mut query_parts: Vec<String> = Vec::new();
    for a in args {
        if let Some(m) = a.strip_prefix("--mode=") {
            mode = Some(match m {
                "ai" => SearchMode::Ai,
                "files" => SearchMode::Files,
                "apps" => SearchMode::Apps,
                other => {
                    eprintln!("unknown --mode={other}; valid: ai, files, apps");
                    std::process::exit(64);
                }
            });
        } else {
            query_parts.push(a.clone());
        }
    }
    (mode, query_parts.join(" "))
}
