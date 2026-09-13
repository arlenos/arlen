//! `arlen-sentineld` - the physical-world privacy sentinel.
//!
//! It answers what this machine broadcasts about itself and holds the switches
//! for the detectors that watch for it (`privacy-sentinel-plan.md`). Settings has
//! been asking for this since 26 August and getting nothing, and the page above
//! it was written to tell somebody they are protected, so an unanswered ask there
//! is not a blank card - it is a protection page with no protection behind it.

use std::sync::Arc;

use arlen_sentineld::config;
use arlen_sentineld::server::{bind, run, socket_path, Context};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                // Both crates this daemon is made of: a target roots at the crate
                // the line was compiled into, so naming one leaves the other mute.
                tracing_subscriber::EnvFilter::new("warn,arlen_sentineld=info")
            }),
        )
        .init();

    let Some(config_path) = config::config_path() else {
        tracing::error!("neither XDG_CONFIG_HOME nor HOME is set, so there is nowhere to keep the detector switches");
        return std::process::ExitCode::FAILURE;
    };
    let socket = socket_path();
    let listener = match bind(&socket) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("cannot bind {}: {e}", socket.display());
            return std::process::ExitCode::FAILURE;
        }
    };
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("cannot listen for SIGTERM: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    // Lazy by construction: an audit daemon that is not up yet is a failed submit
    // at the moment somebody switches a detector off, which is the fail-closed
    // answer rather than a reason to refuse to start.
    let ctx = Arc::new(Context {
        config_path,
        audit: Arc::new(audit_proto::sink::LedgerAuditSink::at_default_socket()),
    });

    // Give back what is past its window before serving anything. A correlator
    // cannot be chained across its rotation, so a tag older than the detection
    // window can never contribute to an alert again and holding its positions
    // would be collecting location history for nothing - which is the thing this
    // daemon exists to object to when other software does it. Best-effort: a store
    // that will not open is a sentinel that cannot yet detect, not one that must
    // refuse to answer what the machine broadcasts.
    prune_sightings();
    // The finder-tag watch, when the person has it on. Spawned rather than
    // awaited: this daemon's first job is answering what the machine broadcasts,
    // and a radio that will not start must not take the socket down with it.
    let tracker = spawn_tracker_watch(&ctx.config_path);
    let recording = spawn_recording_watch(&ctx.config_path);

    tracing::info!(socket = %socket.display(), "privacy sentinel listening");
    tokio::select! {
        () = run(listener, Arc::clone(&ctx)) => {}
        _ = tokio::signal::ctrl_c() => tracing::info!("interrupted, shutting down"),
        _ = term.recv() => tracing::info!("asked to stop, shutting down"),
    }
    tracker.abort();
    recording.abort();
    let _ = std::fs::remove_file(&socket);
    std::process::ExitCode::SUCCESS
}

/// Drop every tracker-sentinel record whose window has closed.
///
/// Separated from `main` so the reason reads at the call site and the handling of
/// each outcome reads here. Nothing is fatal: no state directory means no store to
/// prune, and a store that fails to open is worth a line in the log rather than a
/// daemon that will not start.
fn prune_sightings() {
    use arlen_sentineld::sightings::{SightingStore, StoreError};

    let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs(),
        // A clock before 1970 cannot say what is expired, and guessing would delete
        // somebody's live evidence. Keep everything and try again next start.
        Err(_) => return,
    };
    match SightingStore::open_default() {
        Ok(store) => match store.prune(now) {
            Ok(0) => {}
            Ok(n) => tracing::info!(count = n, "dropped tracker sightings past their window"),
            Err(e) => tracing::warn!("could not prune the sighting store: {e}"),
        },
        Err(StoreError::NoStateDir) => {}
        Err(e) => tracing::warn!("the sighting store did not open: {e}"),
    }
}

/// Start the finder-tag watch if the tracker detector is switched on.
///
/// Reads the switch here rather than inside the watch so the reason for not
/// watching is said once, at the point somebody would look for it. A detector
/// that is off is not an error and not a warning: it is the state the person
/// chose.
fn spawn_tracker_watch(config_path: &std::path::Path) -> tokio::task::JoinHandle<()> {
    use arlen_sentineld::config::{self, Detector};
    use arlen_sentineld::sightings::SightingStore;

    let (cfg, _) = config::load(config_path);
    let tracker = cfg.get(Detector::Tracker).clone();
    tokio::spawn(async move {
        if !tracker.on {
            tracing::info!("the tracker detector is off, so nothing is watched for");
            return;
        }
        let store = match SightingStore::open_default() {
            Ok(store) => store,
            Err(e) => {
                tracing::warn!("no sighting store, so tags cannot be followed: {e}");
                return;
            }
        };
        let connection = match zbus::Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("no system bus, so the radio cannot be reached: {e}");
                return;
            }
        };
        let sensitivity = config::proximity_sensitivity(&tracker);
        // Runs until the task is aborted at shutdown.
        if let Err(e) =
            arlen_sentineld::ble::watch(&store, sensitivity, &connection, std::future::pending())
                .await
        {
            tracing::warn!("the tag watch stopped: {e}");
        }
    })
}

/// Start the nearby-recording watch if that detector is switched on.
///
/// Its own task rather than a branch inside the tag watch: the two detectors have
/// separate switches, and somebody who wants to know about cameras but not about
/// tags should get exactly that.
fn spawn_recording_watch(config_path: &std::path::Path) -> tokio::task::JoinHandle<()> {
    use arlen_sentineld::config::{self, Detector};

    let (cfg, _) = config::load(config_path);
    let recording = cfg.get(Detector::Recording).clone();
    tokio::spawn(async move {
        if !recording.on {
            tracing::info!("the recording detector is off, so nothing is watched for");
            return;
        }
        let classes = arlen_sentinel_detect::recording::bundled_device_classes();
        let sensitivity = config::proximity_sensitivity(&recording);
        if let Err(e) =
            arlen_sentineld::ble::watch_recording(sensitivity, &classes, std::future::pending()).await
        {
            tracing::warn!("the recording watch stopped: {e}");
        }
    })
}
