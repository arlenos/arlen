//! `arlen-auditd` entry point.
//!
//! The daemon opens the append-only ledger, verifies its hash chain,
//! and serves two sockets: the peer-authenticated ingest socket and
//! the read API.
//!
//! Startup runs two independent integrity witnesses: the HMAC hash
//! chain (catches edits, insertions, reordering) and the head
//! checkpoint (catches truncation — deleted rows or a deleted
//! database, which leave a valid shorter prefix the chain alone
//! cannot flag).
//!
//! If either finds tampering, the daemon does not exit — that would
//! crash-loop and take the read API down with it. Instead it freezes
//! ingest (every append is refused, so callers fail closed per
//! foundation §8.4.6), keeps the read API up so the tampered ledger
//! can still be inspected, and emits an `audit.tampered` event on the
//! Event Bus for the Anomaly Detector and the shell.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arlen_auditd::checkpoint::{self, Checkpoint, StartupCheck};
use arlen_auditd::ingest::{ingest_socket_path, IngestServer};
use arlen_auditd::ledger::{Ledger, LedgerReader};
use arlen_auditd::read::{read_socket_path, ReadServer};
use arlen_auditd::{audit_data_dir, key, AuditError};
use os_sdk::{EventEmitter, UnixEventEmitter};
use tokio::sync::Mutex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Identify the daemon as the source of the `audit.*` events it
    // emits; without this the Event Bus envelope source is empty.
    if std::env::var_os("ARLEN_APP_ID").is_none() {
        std::env::set_var("ARLEN_APP_ID", "audit-daemon");
    }
    tracing::info!("arlen-auditd starting");

    let ledger_path = audit_data_dir()?.join("ledger.db");

    // Genesis vs. fault: a missing key file is only acceptable when
    // the ledger is empty too. `probe_has_entries` propagates any
    // database error rather than reading as empty.
    let has_entries = Ledger::probe_has_entries(&ledger_path).await?;
    let key = key::load_or_create(&key::key_path()?, has_entries)?;

    let ledger = Ledger::open(&ledger_path, key).await?;

    // The Event Bus producer client. Created before verification so a
    // tamper alert can be emitted the moment it is detected.
    let producer_socket = std::env::var("ARLEN_PRODUCER_SOCKET")
        .unwrap_or_else(|_| "/run/arlen/event-bus-producer.sock".to_string());
    let emitter = Arc::new(UnixEventEmitter::new(producer_socket));

    // Startup integrity witness 1: the HMAC hash chain.
    let tampered = Arc::new(AtomicBool::new(false));
    match ledger.verify().await {
        Ok(count) => tracing::info!(entries = count, "audit chain verified"),
        Err(AuditError::ChainBroken { index, detail }) => {
            tracing::error!(
                break_index = index,
                "AUDIT LEDGER TAMPERED at index {index} ({detail}); \
                 freezing ingest, read API stays up for inspection"
            );
            tampered.store(true, Ordering::SeqCst);
            emit_tampered(&emitter, &format!("chain broken at index {index}: {detail}"))
                .await;
        }
        // A storage-level failure means the ledger is unreadable, not
        // tampered. The daemon cannot operate; it exits and lets
        // systemd restart it.
        Err(other) => return Err(other.into()),
    }

    // Startup integrity witness 2: the head checkpoint. The chain
    // cannot detect a truncation — deleting the newest rows or the
    // whole database leaves a valid shorter prefix (or a clean
    // genesis) that verifies fine. The checkpoint, written outside the
    // database after every append, catches that. A corrupt or missing
    // checkpoint guarding a non-empty ledger is itself treated as
    // tampering (the witness was destroyed), never silently re-seeded.
    // Skipped when the chain already failed (ingest already frozen).
    if !tampered.load(Ordering::SeqCst) {
        let cp_path = ledger.checkpoint_path().to_path_buf();
        let head = ledger.head_for_checkpoint();
        let stored = checkpoint::read(&cp_path);
        // When a checkpoint is present, look up the ledger's hash at
        // the index it points at — the entry must still be there and
        // unchanged for the witness to hold.
        let entry_hash_at_cp = match &stored {
            Ok(Some(cp)) => ledger.entry_hash_hex_at(cp.index).await?,
            _ => None,
        };
        match checkpoint::assess_startup(stored, head.is_none(), entry_hash_at_cp) {
            StartupCheck::Consistent => {
                // Reseed to the live head: refreshes the witness and
                // advances past a clean crash-ahead entry. This write
                // is mandatory, not best-effort — if it fails the
                // witness would stay stale while the daemon serves,
                // leaving the entries above it unwitnessed. So a
                // failed reseed freezes ingest, exactly as an
                // append-time checkpoint failure does: the daemon
                // cannot keep its witness current, so it must not
                // accept new entries.
                if let Some((index, entry_hash_hex)) = head {
                    if let Err(e) =
                        checkpoint::write(&cp_path, &Checkpoint { index, entry_hash_hex })
                    {
                        tracing::error!(
                            "head checkpoint could not be refreshed at startup ({e}); \
                             the witness is unwritable, freezing ingest"
                        );
                        tampered.store(true, Ordering::SeqCst);
                        emit_tampered(
                            &emitter,
                            &format!("checkpoint witness unwritable at startup: {e}"),
                        )
                        .await;
                    }
                }
            }
            StartupCheck::Genesis => {
                // Empty ledger; the first append writes the checkpoint.
            }
            StartupCheck::Tampered { detail } => {
                tracing::error!(
                    "AUDIT LEDGER TAMPERED ({detail}); freezing ingest, \
                     read API stays up for inspection"
                );
                tampered.store(true, Ordering::SeqCst);
                emit_tampered(&emitter, &detail).await;
            }
        }
    }

    // A separate read-only handle backs the read API, so range
    // queries run concurrently with appends (WAL) and never contend
    // on the writer's lock.
    let reader = Arc::new(LedgerReader::open(&ledger_path).await?);
    let ledger = Arc::new(Mutex::new(ledger));

    let ingest = Arc::new(IngestServer::new(ledger, emitter, tampered.clone()));
    let read = Arc::new(ReadServer::new(reader, tampered));

    // The socket paths are bound to locals so they outlive the
    // futures the `select!` holds, and stay available for cleanup.
    let ingest_path = ingest_socket_path();
    let read_path = read_socket_path();

    // Announce readiness to systemd (`Type=notify`). A no-op when not
    // run under systemd; a failure is logged, never fatal.
    if let Err(err) = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]) {
        tracing::info!(
            "sd_notify ready not sent ({err}); running without systemd readiness"
        );
    }
    tracing::info!("arlen-auditd serving (ingest + read sockets)");

    // Serve both sockets until an accept loop fails or a shutdown
    // signal arrives.
    tokio::select! {
        r = ingest.run(&ingest_path) => r?,
        r = read.run(&read_path) => r?,
        _ = shutdown_signal() => {
            tracing::info!("arlen-auditd: shutdown signal received");
        }
    }

    // Best-effort socket cleanup so the next start's stale-socket
    // probe has nothing to clear.
    let _ = std::fs::remove_file(&ingest_path);
    let _ = std::fs::remove_file(&read_path);
    Ok(())
}

/// Emit a best-effort `audit.tampered` event. The ledger is the
/// source of truth; a bus failure does not change the fail-closed
/// ingest state, so the emit is never awaited for success.
async fn emit_tampered(emitter: &UnixEventEmitter, detail: &str) {
    let payload = serde_json::to_vec(&serde_json::json!({ "detail": detail }))
        .unwrap_or_default();
    let _ = emitter.emit("audit.tampered", payload).await;
}

/// Resolve when the process receives SIGTERM (a systemd stop) or
/// SIGINT (Ctrl-C).
async fn shutdown_signal() {
    let mut term = match tokio::signal::unix::signal(
        tokio::signal::unix::SignalKind::terminate(),
    ) {
        Ok(sig) => sig,
        Err(err) => {
            tracing::warn!("cannot install SIGTERM handler: {err}");
            // Fall back to Ctrl-C only.
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
