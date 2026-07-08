//! The account-config watcher that drives directed change notifications (online-
//! accounts-plan.md §3.1 point 2). It watches `~/.config/arlen/accounts/`, and on
//! a change reloads the account set, diffs it against the previous snapshot, and
//! UNICASTs an `AccountsChanged` signal to each granted app's connection - never a
//! session-wide broadcast. The no-leak decision (who receives what) is the tested
//! pure core in [`crate::presence`]; this module only wires the filesystem event
//! to that decision and the D-Bus emit.

use crate::config::{load_accounts, AccountConfig};
use crate::presence::{diff_accounts, emit_targets, PeerRegistry};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use zbus::Connection;

/// The interface, object path and member the directed signal is sent on.
const ACCOUNTS_PATH: &str = "/org/arlen/Accounts1";
const ACCOUNTS_IFACE: &str = "org.arlen.Accounts1";
const CHANGED_SIGNAL: &str = "AccountsChanged";

/// Emit the directed `AccountsChanged` signal to each target, unicast to the
/// recipient's unique bus name (never broadcast). Best-effort: a failed send to
/// one recipient (its connection has since dropped) is logged and skipped, never
/// stopping the others. The body is `(account_id, change_kind)`.
async fn emit_to_targets(conn: &Connection, targets: &[(String, &str, &'static str)]) {
    for (destination, account_id, kind) in targets {
        if let Err(e) = conn
            .emit_signal(
                Some(destination.as_str()),
                ACCOUNTS_PATH,
                ACCOUNTS_IFACE,
                CHANGED_SIGNAL,
                &(*account_id, *kind),
            )
            .await
        {
            tracing::debug!(%destination, %account_id, error = %e, "directed AccountsChanged emit skipped");
        }
    }
}

/// Watch the account-config directory and unicast `AccountsChanged` to granted
/// apps' connections on every change. Returns if the watch cannot be established
/// or ends (the daemon still serves its methods; only live notifications stop).
pub async fn run_account_watcher(
    conn: Connection,
    accounts_dir: PathBuf,
    peers: Arc<Mutex<PeerRegistry>>,
) {
    use notify::{RecommendedWatcher, RecursiveMode, Watcher};

    // notify calls its handler on its own thread; bridge to async via a channel.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(16);
    let mut watcher = match RecommendedWatcher::new(
        move |_res| {
            let _ = tx.blocking_send(());
        },
        notify::Config::default(),
    ) {
        Ok(w) => w,
        Err(e) => {
            tracing::warn!(error = %e, "account watcher unavailable; live notifications disabled");
            return;
        }
    };
    if let Err(e) = watcher.watch(&accounts_dir, RecursiveMode::NonRecursive) {
        tracing::warn!(dir = %accounts_dir.display(), error = %e, "cannot watch accounts dir");
        return;
    }
    tracing::info!(dir = %accounts_dir.display(), "watching accounts for directed change notifications");

    let mut last: Vec<AccountConfig> = load_accounts(&accounts_dir).0;
    while rx.recv().await.is_some() {
        let (new, _errs) = load_accounts(&accounts_dir);
        let changes = diff_accounts(&last, &new);
        if changes.is_empty() {
            last = new;
            continue;
        }
        // Resolve recipients under the lock, then drop it before the awaited emit.
        let targets = match peers.lock() {
            Ok(peers) => emit_targets(&changes, &last, &new, &peers),
            Err(_) => Vec::new(),
        };
        emit_to_targets(&conn, &targets).await;
        last = new;
    }
}
