//! Manual test harness for the clipboard SDK + IPC broker
//! peer-auth flow.
//!
//! Build:
//!   cargo build -p os-sdk --example clipboard_test
//!
//! Then run with the desktop-shell already up (start-dev.sh).
//! The binary lives at `target/debug/examples/clipboard_test`
//! so identity resolution maps it to `dev.clipboard_test` (per
//! `sdk/permissions::identity` debug fallback). Drop a profile
//! at `~/.config/permissions/dev.clipboard_test.toml` to grant
//! scopes — without it, every operation hits PermissionDenied.
//!
//! Subcommands:
//!   write      — writes "hello from clipboard_test" via SDK
//!   read       — reads current clipboard
//!   subscribe  — subscribes for 30 s, prints each event
//!   history    — fetches up to 10 history entries
//!
//! What this exercises:
//! - Connection-time SO_PEERCRED + identity resolution
//!   (broker logs "connection from app_id=... pid=...")
//! - Per-request scope check
//! - Audit log emission (visible in shell log)

use std::env;

use os_sdk::clipboard::{ClipboardLabel, UnixClipboardClient, WriteParams};

#[tokio::main]
async fn main() {
    let cmd = env::args().nth(1).unwrap_or_default();
    let client = match UnixClipboardClient::connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("connect failed: {e}");
            eprintln!(
                "hint: is the desktop-shell running? socket lives at \
                 $XDG_RUNTIME_DIR/lunaris/clipboard.sock"
            );
            std::process::exit(2);
        }
    };

    match cmd.as_str() {
        "write" => {
            let res = client
                .write(WriteParams {
                    content: b"hello from clipboard_test".to_vec(),
                    mime: "text/plain".into(),
                    label: ClipboardLabel::Normal,
                })
                .await;
            match res {
                Ok(()) => println!("write: OK"),
                Err(e) => {
                    eprintln!("write FAILED: {e}");
                    std::process::exit(1);
                }
            }
        }
        "read" => {
            match client.read().await {
                Ok(Some(entry)) => {
                    println!(
                        "read: id={} label={:?} mime={} content={}",
                        entry.id,
                        entry.label,
                        entry.mime,
                        entry
                            .content
                            .as_ref()
                            .map(|b| String::from_utf8_lossy(b).into_owned())
                            .unwrap_or_else(|| "<stripped>".to_string())
                    );
                }
                Ok(None) => println!("read: OK (empty clipboard)"),
                Err(e) => {
                    eprintln!("read FAILED: {e}");
                    std::process::exit(1);
                }
            }
        }
        "subscribe" => {
            println!("subscribe: 30s window, Ctrl-C to stop");
            let mut rx = match client.subscribe().await {
                Ok(rx) => rx,
                Err(e) => {
                    eprintln!("subscribe FAILED: {e}");
                    std::process::exit(1);
                }
            };
            let deadline = std::time::Instant::now()
                + std::time::Duration::from_secs(30);
            loop {
                match tokio::time::timeout(
                    deadline.saturating_duration_since(std::time::Instant::now()),
                    rx.recv(),
                )
                .await
                {
                    Ok(Some(entry)) => {
                        println!(
                            "  event: id={} label={:?} content={}",
                            entry.id,
                            entry.label,
                            entry
                                .content
                                .as_ref()
                                .map(|b| String::from_utf8_lossy(b).into_owned())
                                .unwrap_or_else(|| "<stripped>".to_string())
                        );
                    }
                    Ok(None) => {
                        println!("subscribe: stream closed");
                        return;
                    }
                    Err(_) => {
                        println!("subscribe: 30s elapsed, exiting");
                        return;
                    }
                }
            }
        }
        "history" => {
            match client.history(10).await {
                Ok(entries) => {
                    println!("history: {} entries", entries.len());
                    for e in entries {
                        println!(
                            "  id={} label={:?} content={}",
                            e.id,
                            e.label,
                            e.content
                                .as_ref()
                                .map(|b| String::from_utf8_lossy(b).into_owned())
                                .unwrap_or_else(|| "<stripped>".to_string())
                        );
                    }
                }
                Err(e) => {
                    eprintln!("history FAILED: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: clipboard_test <write|read|subscribe|history>");
            std::process::exit(64);
        }
    }
}
