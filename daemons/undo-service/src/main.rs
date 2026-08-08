// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! `arlen-undod`: the session undo service.
//!
//! Owns `org.arlen.Undo1` on the session bus and serves the recent-actions list
//! and the reversal. It reads two stores that are already independent daemons -
//! the signed undo log and the audit ledger - and holds nothing itself.
//!
//! **It starts and serves regardless of `[ai] enabled`.** That is the whole
//! reason it is a separate process: the same operations lived on
//! `org.arlen.AIAgent1`, which the AI engine registers only when the assistant is
//! switched on, so turning the assistant off in Settings removed a user's own file
//! moves from the list and their undo with them. The records never depended on the
//! assistant. Nothing in this binary may learn to ask whether it is running.

use arlen_undo::iface::{UndoInterface, BUS_NAME, OBJECT_PATH};
use tracing::{info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let connection = match zbus::connection::Builder::session() {
        Ok(builder) => match builder.build().await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "no session bus; undo is unreachable this session");
                return;
            }
        },
        Err(e) => {
            warn!(error = %e, "no session bus address; undo is unreachable this session");
            return;
        }
    };

    let audit: std::sync::Arc<dyn audit_proto::sink::AuditSink> =
        std::sync::Arc::new(audit_proto::sink::LedgerAuditSink::at_default_socket());
    if let Err(e) = connection
        .object_server()
        .at(OBJECT_PATH, UndoInterface { audit })
        .await
    {
        warn!(error = %e, "could not serve the undo surface");
        return;
    }
    // Sole owner, never queued: two processes answering for the same undo would
    // let a caller reach whichever won the race, and a queued second copy would
    // sit silently until the first died. Refuse instead.
    match connection
        .request_name_with_flags(BUS_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await
    {
        Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => {
            info!("serving {BUS_NAME} (session undo)")
        }
        Ok(other) => {
            warn!(?other, "{BUS_NAME} is owned elsewhere; not serving");
            return;
        }
        Err(e) => {
            warn!(error = %e, "could not own {BUS_NAME}");
            return;
        }
    }

    shutdown().await;
    info!("shutting down");
}

/// Wait for SIGINT or SIGTERM. A supervisor stopping the service should end it,
/// not leave it holding the bus name.
async fn shutdown() {
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "no SIGTERM handler; waiting on ctrl-c alone");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
