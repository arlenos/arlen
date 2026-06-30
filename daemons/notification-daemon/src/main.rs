/// Arlen Notification Daemon.
///
/// Owns `org.freedesktop.Notifications` on the session D-Bus. Stores
/// notifications in SQLite, enforces DND rules, and broadcasts to
/// connected shell clients via a Unix socket.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Mutex;
use zbus::connection;

use arlen_notification_daemon::config;
use arlen_notification_daemon::dbus::NotificationServer;
use arlen_notification_daemon::events;
use arlen_notification_daemon::manager::NotificationManager;
use arlen_notification_daemon::socket::SocketServer;
use arlen_notification_daemon::storage::Database;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Self-confine before the runtime starts (Tier-A #2). The daemon writes
    // only its SQLite history db (<data>/arlen), the synthesized fallback sound
    // theme (<data>/sounds, the freedesktop sound-theme dir) and its shell
    // socket; the session bus and the per-app audit submit are connects, not
    // path writes, so incoming notification content (an untrusted-app surface)
    // can never escape these dirs. Pre-create the three dirs so their write
    // grants are expressible, then fence on the main thread BEFORE the runtime
    // so every worker and spawned task inherits the domain. The daemon spawns
    // tokio tasks, not child processes, so there is no inherited-domain trap.
    let db_dir = data_root().join("arlen");
    let sounds_root = data_root().join("sounds");
    let socket_path = SocketServer::default_path();
    let socket_dir = socket_path.parent().map(Path::to_path_buf);
    let _ = std::fs::create_dir_all(&db_dir);
    let _ = std::fs::create_dir_all(&sounds_root);
    if let Some(d) = &socket_dir {
        let _ = std::fs::create_dir_all(d);
    }
    apply_fence(&db_dir, &sounds_root, socket_dir.as_deref());

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())
}

/// The data-dir root the daemon writes under (`$XDG_DATA_HOME`, else
/// `~/.local/share`, else `/tmp`), matching the fallback the db and sounds
/// paths use so the fence grant and the actual writes resolve identically.
fn data_root() -> std::path::PathBuf {
    dirs::data_dir().unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
}

/// Install the Landlock write-fence over the daemon's write dirs (the history
/// db dir, the synth sound-theme dir and the socket dir). Defense-in-depth: a
/// kernel that cannot enforce it leaves the daemon exactly as safe as no fence,
/// so by default a non-enforcing kernel or a ruleset error is logged and the
/// daemon continues. A hardened deployment that wants the confinement
/// guaranteed sets `ARLEN_NOTIFICATION_REQUIRE_FENCE=1`, making a non-enforcing
/// kernel a fatal startup error.
fn apply_fence(db_dir: &Path, sounds_root: &Path, socket_dir: Option<&Path>) {
    use arlen_landlock_fence::{fence_writes, FenceOutcome};
    let require =
        std::env::var_os("ARLEN_NOTIFICATION_REQUIRE_FENCE").is_some_and(|v| v == "1");
    let mut writable: Vec<&Path> = vec![db_dir, sounds_root];
    if let Some(d) = socket_dir {
        writable.push(d);
    }
    let degraded = match fence_writes(&writable) {
        Ok(FenceOutcome::Enforced) => {
            tracing::info!("landlock write-fence enforced (write-confined to db + sounds + socket dirs)");
            None
        }
        Ok(FenceOutcome::NotEnforced) => Some("landlock not enforced by this kernel".to_string()),
        Err(e) => Some(format!("landlock fence not applied: {e}")),
    };
    if let Some(reason) = degraded {
        if require {
            tracing::error!(
                "ARLEN_NOTIFICATION_REQUIRE_FENCE=1 but the fence is not active ({reason}); refusing to run unconfined"
            );
            std::process::exit(1);
        }
        tracing::warn!("{reason}; running unconfined (no worse than no fence)");
    }
}

/// The async serve body: load config, open the db, claim the session-bus name,
/// start the socket server + watchers, render the fallback sound theme, then
/// wait for shutdown. Runs entirely inside the write-fence installed by
/// [`main`].
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing::info!("starting notification daemon");

    // 1. Load config.
    let config_path = config::default_config_path();
    let cfg = config::load_config(&config_path);
    let config = Arc::new(Mutex::new(cfg));

    // 2. Init database.
    let db_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("arlen");
    let _ = std::fs::create_dir_all(&db_dir);
    let db_file = db_dir.join("notifications.db");
    let db_path = format!("sqlite:{}?mode=rwc", db_file.display());
    tracing::info!("opening database: {db_path}");
    let db = Arc::new(Database::open(&db_path).await?);
    tracing::info!("database opened at {db_path}");

    // 3. Create manager. The D-Bus server delegates every incoming
    // notify() call to `manager.handle_notify()` so DND / rate limits /
    // SQLite storage all run in one place.
    let (dbus_server, event_rx) = NotificationServer::new();
    let event_tx = dbus_server.event_sender();
    let mut manager_builder = NotificationManager::new(db.clone(), config.clone(), event_tx.clone())
        // GAP-2: each handled notification submits a content-free record
        // (posting app + disposition, never the message) to arlen-auditd.
        .with_audit(Arc::new(audit_proto::LedgerAuditSink::at_default_socket()));
    // Play cues through the system audio CLI when one is installed; otherwise
    // keep the headless logging player (the cue pipeline still runs).
    match arlen_notification_daemon::sound::SystemSoundPlayer::discover() {
        Some(player) => {
            tracing::info!("sound cues will play via the system audio CLI");
            manager_builder = manager_builder.with_sound_player(Arc::new(player));
        }
        None => tracing::info!("no system audio CLI found; sound cues resolve but stay silent"),
    }
    let manager = Arc::new(manager_builder);
    dbus_server.set_manager(manager.clone());

    // 4. Start D-Bus server. Register the interface first, then claim the
    // well-known name explicitly. A name already held by another notification
    // server (a running desktop session's own daemon) is a clean decline, not
    // a crash: this instance stops instead of looping under a supervisor, and
    // declining (rather than replacing) means it never hijacks the session's
    // real notifications. Replacing a competing owner would be a policy change,
    // not this guard's job.
    use zbus::fdo::{RequestNameFlags, RequestNameReply};
    let _conn = connection::Builder::session()?
        .serve_at("/org/freedesktop/Notifications", dbus_server)?
        .build()
        .await?;
    match _conn
        .request_name_with_flags(
            "org.freedesktop.Notifications",
            RequestNameFlags::DoNotQueue.into(),
        )
        .await
    {
        Ok(RequestNameReply::PrimaryOwner) | Ok(RequestNameReply::AlreadyOwner) => {
            tracing::info!("D-Bus server ready");
        }
        // Under DoNotQueue, zbus maps a name owned by another peer to
        // Err(NameTaken) (the fdo reply is Exists); a non-owning Ok reply is
        // the defensive fallback. Either way decline cleanly.
        Err(zbus::Error::NameTaken) => {
            tracing::warn!(
                "org.freedesktop.Notifications is owned by another notification server; this instance will stop to avoid hijacking session notifications"
            );
            return Ok(());
        }
        Ok(other) => {
            tracing::warn!(?other, "org.freedesktop.Notifications not acquired; this instance will stop");
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    }

    // 5. Start socket server in background.
    let socket_path = SocketServer::default_path();
    let socket_server = SocketServer::new(socket_path);
    let dnd_mode = manager.dnd_mode();
    tokio::spawn(async move {
        if let Err(e) = socket_server.start(event_rx, event_tx, db.clone(), dnd_mode).await {
            tracing::error!("socket server error: {e}");
        }
    });

    // 6. Start config watcher.
    let config_for_watcher = config.clone();
    if let Ok((mut config_rx, _watcher)) =
        config::watcher::watch_config(config_path)
    {
        tokio::spawn(async move {
            while let Ok(new_config) = config_rx.recv().await {
                *config_for_watcher.lock().await = new_config;
                tracing::info!("config hot-reloaded");
            }
        });
    }

    // 7. Event Bus consumer: subscribe to `focus.*` and
    // `window.fullscreen_*` events so the DND state machine updates
    // when the shell enters/leaves Focus Mode or the compositor
    // enters/leaves fullscreen. Failures log and retry — the daemon
    // must keep working if the Event Bus is down.
    events::consumer::start(manager.clone());

    // 8. Retention cleanup task (runs daily).
    let manager_for_cleanup = manager.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(86400)).await;
            manager_for_cleanup.cleanup().await;
        }
    });

    // Run initial cleanup on startup.
    manager.cleanup().await;

    // 9. Ensure the zero-asset default synth sound theme exists under the user's
    // sounds dir (idempotent: it never clobbers a prior render or customization), so
    // the sound resolver always has a usable fallback cue set even with no sample
    // theme installed. Best-effort - a failure here never blocks the daemon.
    let sounds_root = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("sounds");
    match arlen_notification_daemon::synth::ensure_default_synth_theme(&sounds_root, 48_000) {
        Ok(true) => {
            tracing::info!("rendered the default synth sound theme at {}", sounds_root.display())
        }
        Ok(false) => {}
        Err(e) => tracing::warn!("could not render the default synth sound theme: {e}"),
    }

    tracing::info!("notification daemon ready");

    // Wait for shutdown signal.
    tokio::signal::ctrl_c().await?;
    tracing::info!("shutting down");

    Ok(())
}
