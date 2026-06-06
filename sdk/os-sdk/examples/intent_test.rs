//! Manual test harness for the `shell.intents.dispatch` SDK + IPC
//! broker.
//!
//! Build:
//!   cargo build -p os-sdk --example intent_test
//!
//! Run with the desktop-shell already up. The binary lives at
//! `target/debug/examples/intent_test` so identity resolution maps
//! it to `dev.intent_test`. Drop a profile at
//! `~/.config/permissions/dev.intent_test.toml`:
//!
//!   [info]
//!   app_id = "dev.intent_test"
//!   tier = "first-party"
//!
//!   [intents]
//!   dispatch = true
//!
//! Without that file, the broker returns PermissionDenied for
//! every request.
//!
//! Subcommands:
//!   url <url>             open URL with default app
//!   file <abs-path>       open file with default app
//!   text <body>           write text to clipboard
//!   email <mailto-uri>    open mailto: URI
//!   project <project_id>  activate Focus Mode for project
//!
//! What this exercises:
//! - SO_PEERCRED + identity resolution (broker logs
//!   "connection from app_id=dev.intent_test pid=...")
//! - Per-request `[intents] dispatch` scope check
//! - Type-specific built-in dispatch
//! - Audit log emission (verify NO data content appears in audit
//!   lines — the leak guard)
//! - `app.intent.dispatched` Event-Bus emission with hashed-
//!   subject (subject = type, NOT the data)

use std::env;

use os_sdk::intents::{IntentType, UnixIntentClient};

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("usage: intent_test <url|file|text|email|project> <data>");
        std::process::exit(64);
    }

    let kind = match args[0].as_str() {
        "url" => IntentType::Url,
        "file" => IntentType::File,
        "text" => IntentType::Text,
        "email" => IntentType::Email,
        "project" => IntentType::Project,
        other => {
            eprintln!("unknown intent type: {other}");
            std::process::exit(64);
        }
    };

    let data = args[1..].join(" ");

    let client = match UnixIntentClient::new() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("client construct failed: {e}");
            std::process::exit(2);
        }
    };

    match client.dispatch("view", kind, data.as_bytes(), None).await {
        Ok(r) => {
            println!(
                "dispatch: OK (handler={:?} outcome={:?})",
                r.handler, r.outcome
            );
        }
        Err(e) => {
            eprintln!("dispatch FAILED: {e}");
            std::process::exit(1);
        }
    }
}
