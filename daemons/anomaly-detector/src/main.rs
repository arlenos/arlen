//! `arlen-anomalyd` entry point.
//!
//! Resolves the per-user data dir, loads the persisted baseline,
//! wires the audit read client and the Event Bus consumer, and runs
//! the detector. Advisory only: it raises notifications, it never
//! blocks AI activity (foundation §8.4.8).

#![deny(unsafe_code)]

use audit_proto::{read_socket_path, ReadClient};
use arlen_anomalyd::detect::DetectorConfig;
use arlen_anomalyd::source::Detector;
use arlen_anomalyd::state::State;
use arlen_anomalyd::{data_dir, ensure_private_dir, now_micros};
use os_sdk::UnixEventConsumer;

/// Startup grace: the no-interaction check does not fire within this
/// window of start, since we may not have observed user activity yet.
const STARTUP_GRACE_SECS: i64 = 120;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("arlen-anomalyd starting");

    let dir = data_dir()?;
    ensure_private_dir(&dir)?;
    let state_path = dir.join("state.json");
    let state = State::load(&state_path);
    tracing::info!(
        hwm = state.hwm_index,
        known_types = state.novelty.known.len(),
        "loaded anomaly state"
    );

    let read = ReadClient::new(read_socket_path());
    let grace_until = now_micros() + STARTUP_GRACE_SECS * 1_000_000;
    let detector = Detector::new(
        state,
        DetectorConfig::default(),
        Box::new(read),
        state_path,
        grace_until,
        true, // dispatch notifications over D-Bus
    );

    let consumer = UnixEventConsumer::new(resolve_event_consumer_socket());

    // Announce readiness to systemd (no-op when not run under it).
    if let Err(err) = sd_notify::notify(false, &[sd_notify::NotifyState::Ready]) {
        tracing::info!("sd_notify ready not sent ({err}); running without systemd readiness");
    }
    tracing::info!("arlen-anomalyd running");

    // The detector loops forever; a shutdown signal cancels it. State
    // is flushed every poll cycle, so cancelling mid-run loses at most
    // the last cycle's updates (re-derived on the next start).
    tokio::select! {
        _ = detector.run(consumer) => {}
        _ = shutdown_signal() => tracing::info!("arlen-anomalyd: shutdown signal received"),
    }
    Ok(())
}

/// Resolve the Event Bus consumer socket: explicit
/// `LUNARIS_CONSUMER_SOCKET` wins; else the per-user runtime path when
/// present; else the system path.
fn resolve_event_consumer_socket() -> String {
    if let Ok(explicit) = std::env::var("LUNARIS_CONSUMER_SOCKET") {
        if !explicit.is_empty() {
            return explicit;
        }
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        if !xdg.is_empty() {
            let runtime = format!("{xdg}/arlen/event-bus-consumer.sock");
            if std::path::Path::new(&runtime).exists() {
                return runtime;
            }
        }
    }
    "/run/arlen/event-bus-consumer.sock".to_string()
}

/// Resolve on SIGTERM (systemd stop) or SIGINT (Ctrl-C).
async fn shutdown_signal() {
    let mut term =
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(sig) => sig,
            Err(err) => {
                tracing::warn!("cannot install SIGTERM handler: {err}");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
