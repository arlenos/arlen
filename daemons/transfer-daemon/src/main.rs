//! The Arlen transfer daemon (`arlen-transferd`).
//!
//! Loads the directional transfer policy (fail-closed: a broken policy is the
//! sealed default-deny set) and stands up the cross-profile broker. The policy
//! gate is the security boundary: every request passes caller admission, request
//! validation and the directional default-deny policy, and every decision is
//! audited to BOTH profiles' ledgers before any byte crosses a profile boundary.
//!
//! This CORE wires the testable pieces and reports the loaded policy. The live
//! per-uid request listeners and the dual-uid byte-moving broker are the
//! on-system wiring (they need PR-R1's per-uid sockets and two live profile
//! uids); the daemon holds the fail-closed [`DeniedBroker`] until they land, so
//! no transfer can move a byte even if a request surface is added before the
//! live broker.

use transfer_daemon::broker::{DeniedBroker, TransferBroker};
use transfer_daemon::config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let dir = config::transfer_dir().ok_or("no config home for the transfer policy")?;
    let (policy, error) = config::load_policy(&dir);
    if let Some(err) = error {
        // A malformed policy is reported and replaced by the sealed default-deny
        // set, never granted.
        tracing::warn!(dir = %dir.display(), %err, "transfer policy failed to load; sealing (default-deny)");
    }
    tracing::info!(
        rules = policy.rules.len(),
        dir = %dir.display(),
        "transfer policy loaded; the directional gate mediates every transfer",
    );

    // The cross-uid byte-moving broker is fail-closed until the live dual-uid
    // impl lands (it needs PR-R1's per-uid sockets and two live profile uids).
    // A daemon wired with the stand-in audits the gate decision and then refuses
    // delivery, never silently succeeding.
    let _broker: Box<dyn TransferBroker> = Box::new(DeniedBroker);
    tracing::info!(
        "cross-profile delivery is fail-closed (DeniedBroker); the live dual-uid broker lands with PR-R1",
    );

    // Serve until terminated. The per-uid request listeners + the live broker
    // are the on-system wiring this CORE deliberately defers.
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");
    Ok(())
}
