/// Central notification manager.
///
/// Coordinates between D-Bus input, DND evaluation, rate limiting,
/// storage, and client broadcasting.

use std::path::PathBuf;
use std::sync::Arc;

use audit_proto::AuditSink;
use tokio::sync::{broadcast, Mutex};

use crate::audit::notification_event;
use crate::config::{Config, DndMode};
use crate::dbus::server::{
    clamp_critical, determine_priority, CloseReason, Notification, NotifyEvent,
};
use crate::dnd::{DndState, SuppressResult};
use crate::manager::grouping::derive_group_key;
use crate::manager::rate_limiter::RateLimiter;
use crate::manager::validation::sanitize_input;
use crate::sound::{self, NullSoundPlayer, SoundPlayer};
use crate::storage::Database;

/// Central coordinator for the notification daemon.
pub struct NotificationManager {
    db: Arc<Database>,
    dnd_state: Arc<Mutex<DndState>>,
    dnd_mode: Arc<Mutex<DndMode>>,
    config: Arc<Mutex<Config>>,
    rate_limiter: Mutex<RateLimiter>,
    events: broadcast::Sender<NotifyEvent>,
    /// Queued notifications waiting for fullscreen exit.
    fullscreen_queue: Mutex<Vec<Notification>>,
    /// Content-free audit sink (GAP-2). Best-effort: a notification is
    /// observed system activity, not a fail-closed capability exercise, so a
    /// down ledger logs and is skipped rather than dropping the notification.
    /// `None` in tests and when no sink is attached.
    audit: Option<Arc<dyn AuditSink>>,
    /// Notification-cue playback seam. The headless default
    /// ([`NullSoundPlayer`]) runs the resolve + should-play pipeline without an
    /// audio device; the metal PipeWire backend is injected via
    /// [`with_sound_player`](Self::with_sound_player).
    player: Arc<dyn SoundPlayer>,
    /// The `.../sounds` theme base directories searched for a cue's file, in
    /// lookup precedence ([`sound::default_sound_roots`]).
    sound_roots: Vec<PathBuf>,
}

impl NotificationManager {
    /// Create a new notification manager.
    pub fn new(
        db: Arc<Database>,
        config: Arc<Mutex<Config>>,
        events: broadcast::Sender<NotifyEvent>,
    ) -> Self {
        Self {
            db,
            dnd_state: Arc::new(Mutex::new(DndState::default())),
            dnd_mode: Arc::new(Mutex::new(DndMode::Off)),
            config,
            rate_limiter: Mutex::new(RateLimiter::new()),
            events,
            fullscreen_queue: Mutex::new(Vec::new()),
            audit: None,
            player: Arc::new(NullSoundPlayer),
            sound_roots: sound::default_sound_roots(),
        }
    }

    /// Attach the content-free audit sink (GAP-2). The daemon wires the
    /// production `LedgerAuditSink`; left `None` it audits nothing.
    pub fn with_audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = Some(audit);
        self
    }

    /// Replace the headless [`NullSoundPlayer`] with a real backend (the metal
    /// PipeWire player). Left unset, the daemon runs the full cue pipeline but
    /// plays nothing, so cue logic is exercised without an audio device.
    pub fn with_sound_player(mut self, player: Arc<dyn SoundPlayer>) -> Self {
        self.player = player;
        self
    }

    /// Get the shared DND mode reference (for socket server).
    pub fn dnd_mode(&self) -> Arc<Mutex<DndMode>> {
        self.dnd_mode.clone()
    }

    /// Handle an incoming notification from D-Bus.
    ///
    /// `may_set_critical` is the caller's authority to raise a DND-piercing
    /// `Critical` priority (GAP-7), decided from its attested identity by the
    /// D-Bus layer. When false, a determined `Critical` is clamped to `High`.
    ///
    /// Returns the notification ID if stored, or 0 if rate-limited.
    pub async fn handle_notify(
        &self,
        id: u32,
        app_name: &str,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: &[String],
        urgency: u8,
        category: &str,
        expire_timeout: i32,
        may_set_critical: bool,
    ) -> u32 {
        // 1. Validate/sanitize input.
        let input = sanitize_input(app_name, summary, body, app_icon, actions);

        // 2. Rate limit.
        {
            let mut rl = self.rate_limiter.lock().await;
            if !rl.check(&input.app_name) {
                tracing::warn!(
                    app = %input.app_name,
                    "rate limited, dropping notification"
                );
                return 0;
            }
        }

        // 3. Determine priority, then clamp a DND-piercing Critical to High
        //    unless the caller is a trusted system notifier (GAP-7). The clamp
        //    covers Critical however it was reached (urgency 2 or never-expire).
        let priority = clamp_critical(
            determine_priority(urgency, expire_timeout, category),
            may_set_critical,
        );

        // 4. Build notification.
        let notification = Notification {
            id,
            app_name: input.app_name.clone(),
            summary: input.summary,
            body: input.body,
            app_icon: input.app_icon,
            actions: input
                .actions
                .chunks(2)
                .filter_map(|c| {
                    if c.len() == 2 {
                        Some((c[0].clone(), c[1].clone()))
                    } else {
                        None
                    }
                })
                .collect(),
            priority,
            urgency,
            category: category.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            expire_timeout,
            read: false,
        };

        // 5. Derive group key (used by shell for visual grouping).
        let group_key = derive_group_key(&notification);

        // 6. Evaluate DND/app rules BEFORE storage so `enabled = false`
        // and `DndMode::Total` can cleanly drop the notification.
        let (suppress_result, history_enabled, sound_config) = {
            let config = self.config.lock().await;
            let app_override = config.apps.get(&input.app_name).cloned();

            // Mirror current effective mode into the shared Arc so the
            // socket server can answer `GetDnd` without re-reading config.
            let mut mode = self.dnd_mode.lock().await;
            *mode = config.dnd.mode;
            drop(mode);

            let dnd_state = self.dnd_state.lock().await;
            let result =
                dnd_state.should_suppress(&notification, &config.dnd, app_override.as_ref());
            (result, config.history.enabled, config.sound.clone())
        };

        // 7. Persist unless Drop, OR history is disabled entirely.
        tracing::debug!(
            id,
            app = %input.app_name,
            ?suppress_result,
            history_enabled,
            "manager: about to decide storage"
        );
        if history_enabled && suppress_result != SuppressResult::Drop {
            match self.db.insert_notification(&notification).await {
                Ok(_) => tracing::debug!(id, "manager: stored in SQLite"),
                Err(e) => tracing::error!(id, "manager: insert failed: {e}"),
            }
        } else if !history_enabled {
            tracing::debug!(id, "manager: skipped storage (history disabled)");
        } else {
            tracing::debug!(id, "manager: skipped storage (DND drop)");
        }

        // Content-free GAP-2 audit of what the daemon did with this
        // notification (the posting app + the disposition, never the message).
        // Best-effort and off the reply path: spawned so a slow or down ledger
        // never delays the D-Bus return, and a submit error logs rather than
        // dropping the notification. `SuppressResult` is `Copy`, so reading the
        // disposition here does not disturb the dispatch match below.
        if let Some(sink) = self.audit.clone() {
            let outcome = match suppress_result {
                SuppressResult::Allow => "shown",
                SuppressResult::Suppress => "suppressed",
                SuppressResult::Queue => "queued",
                SuppressResult::Drop => "dropped",
            };
            let app = input.app_name.clone();
            tokio::spawn(async move {
                if let Err(e) = sink.submit(notification_event(&app, outcome)).await {
                    tracing::debug!("notification audit submit failed: {e}");
                }
            });
        }

        // 8. Act on result.
        match suppress_result {
            SuppressResult::Allow => {
                tracing::info!(id, %group_key, "notification broadcast");
                let _ = self.events.send(NotifyEvent::Added(notification));
            }
            SuppressResult::Suppress => {
                tracing::debug!(id, %group_key, "notification suppressed by DND");
            }
            SuppressResult::Queue => {
                tracing::debug!(id, %group_key, "notification queued (fullscreen)");
                self.fullscreen_queue.lock().await.push(notification);
            }
            SuppressResult::Drop => {
                tracing::debug!(id, %group_key, "notification dropped (blocked)");
            }
        }

        // 9. Sound cue. The `cue_should_play` gate encodes the whole policy (only
        // a shown notification sounds, never when muted or at zero volume), so
        // the call is unconditional here and the gate decides - keeping the
        // play-or-not decision in the one tested function rather than splitting it
        // across the dispatch arms. A low-urgency notification maps to no cue at
        // all (`sound_event_for_notification` returns `None`). Resolution is cheap
        // filesystem logic; the player's `play` is non-blocking by contract.
        if let Some(event) = sound::sound_event_for_notification(urgency, category) {
            if sound::cue_should_play(suppress_result, sound_config.muted, sound_config.volume) {
                let resolution = sound::resolve_cue(event, &sound_config, &self.sound_roots);
                self.player.play(&resolution);
            }
        }

        id
    }

    /// Handle closing a notification.
    pub async fn handle_close(&self, id: u32, reason: CloseReason) {
        self.db.dismiss(id, reason).await.ok();
        let _ = self.events.send(NotifyEvent::Closed { id, reason });
    }

    /// Set fullscreen state. Flushes queue on exit.
    pub async fn set_fullscreen(&self, active: bool) {
        self.dnd_state.lock().await.fullscreen_active = active;

        if !active {
            self.flush_fullscreen_queue().await;
        }
    }

    /// Activate Focus Mode suppression for a project.
    ///
    /// `suppress_apps` is matched case-insensitively against incoming
    /// `app_name` values during the DND check. Re-activating with a new
    /// project_id replaces the previous state (no additive behaviour),
    /// matching the ephemeral `FocusSuppression` semantics.
    pub async fn activate_focus(&self, project_id: String, suppress_apps: Vec<String>) {
        self.dnd_state
            .lock()
            .await
            .focus
            .activate(project_id, suppress_apps);
    }

    /// Deactivate Focus Mode suppression.
    pub async fn deactivate_focus(&self) {
        self.dnd_state.lock().await.focus.deactivate();
    }

    /// Flush queued notifications (max 5, for fullscreen exit).
    async fn flush_fullscreen_queue(&self) {
        let mut queue = self.fullscreen_queue.lock().await;
        let to_send: Vec<Notification> = queue.drain(..).take(5).collect();
        drop(queue);

        for n in to_send {
            let _ = self.events.send(NotifyEvent::Added(n));
        }
    }

    /// Get unread count.
    pub async fn unread_count(&self) -> u32 {
        self.db.count_pending().await.unwrap_or(0)
    }

    /// Run retention cleanup.
    ///
    /// When `history.enabled = false` the database is wiped on every
    /// tick so nothing ever accumulates. Otherwise the configured
    /// age/count limits are enforced.
    pub async fn cleanup(&self) {
        let (enabled, max_age, max_count) = {
            let config = self.config.lock().await;
            (
                config.history.enabled,
                config.history.max_age_days,
                config.history.max_count,
            )
        };

        if !enabled {
            match self.db.cleanup(0, 0).await {
                Ok(n) if n > 0 => tracing::info!("history disabled: wiped {n} notifications"),
                Ok(_) => {}
                Err(e) => tracing::warn!("history wipe failed: {e}"),
            }
            return;
        }

        match self.db.cleanup(max_age, max_count).await {
            Ok(n) if n > 0 => tracing::info!("retention cleanup: removed {n} notifications"),
            Ok(_) => {}
            Err(e) => tracing::warn!("retention cleanup failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn make_manager() -> (NotificationManager, broadcast::Receiver<NotifyEvent>) {
        let db = Arc::new(Database::open_memory().await.unwrap());
        let config = Arc::new(Mutex::new(Config::default()));
        let (tx, rx) = broadcast::channel(64);
        let mgr = NotificationManager::new(db, config, tx);
        (mgr, rx)
    }

    #[tokio::test]
    async fn test_handle_notify_stores_and_broadcasts() {
        let (mgr, mut rx) = make_manager().await;

        let id = mgr
            .handle_notify(1, "Firefox", "", "Done", "file.zip", &[], 1, "", -1, true)
            .await;
        assert_eq!(id, 1);

        // Should be in DB.
        let n = mgr.db.get_notification(1).await.unwrap().unwrap();
        assert_eq!(n.summary, "Done");

        // Should have broadcast.
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, NotifyEvent::Added(_)));
    }

    #[tokio::test]
    async fn test_handle_notify_dnd_suppresses() {
        let db = Arc::new(Database::open_memory().await.unwrap());
        let mut config = Config::default();
        config.dnd.mode = DndMode::Priority;
        let config = Arc::new(Mutex::new(config));
        let (tx, mut rx) = broadcast::channel(64);
        let mgr = NotificationManager::new(db, config, tx);

        let id = mgr
            .handle_notify(1, "App", "", "Hello", "", &[], 1, "", -1, true)
            .await;
        assert_eq!(id, 1);

        // Should be in DB (stored even if suppressed).
        assert!(mgr.db.get_notification(1).await.unwrap().is_some());

        // Should NOT have broadcast.
        assert!(rx.try_recv().is_err());
    }

    /// A `SoundPlayer` that counts `play` calls, so a test can assert the cue
    /// pipeline fired (or did not) without an audio device.
    struct CountingPlayer(Arc<std::sync::atomic::AtomicUsize>);
    impl SoundPlayer for CountingPlayer {
        fn play(&self, _resolution: &crate::sound::SoundResolution) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn a_shown_normal_notification_plays_a_cue_but_low_urgency_and_muted_do_not() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // Normal urgency, default config (unmuted, full volume): the cue plays.
        let plays = Arc::new(AtomicUsize::new(0));
        let (mgr, _rx) = make_manager().await;
        let mgr = mgr.with_sound_player(Arc::new(CountingPlayer(plays.clone())));
        mgr.handle_notify(1, "App", "", "Hi", "", &[], 1, "", -1, true).await;
        assert_eq!(plays.load(Ordering::SeqCst), 1, "a shown normal notification sounds");

        // Low urgency maps to no cue at all.
        mgr.handle_notify(2, "App", "", "Hi", "", &[], 0, "", -1, true).await;
        assert_eq!(plays.load(Ordering::SeqCst), 1, "low urgency stays silent");

        // Muted globally: the gate refuses even a normal arrival.
        let plays = Arc::new(AtomicUsize::new(0));
        let db = Arc::new(Database::open_memory().await.unwrap());
        let mut config = Config::default();
        config.sound.muted = true;
        let config = Arc::new(Mutex::new(config));
        let (tx, _rx) = broadcast::channel(64);
        let mgr = NotificationManager::new(db, config, tx)
            .with_sound_player(Arc::new(CountingPlayer(plays.clone())));
        mgr.handle_notify(1, "App", "", "Hi", "", &[], 1, "", -1, true).await;
        assert_eq!(plays.load(Ordering::SeqCst), 0, "muted plays nothing");
    }

    #[tokio::test]
    async fn test_handle_notify_critical_bypasses_dnd() {
        let db = Arc::new(Database::open_memory().await.unwrap());
        let mut config = Config::default();
        config.dnd.mode = DndMode::Priority;
        let config = Arc::new(Mutex::new(config));
        let (tx, mut rx) = broadcast::channel(64);
        let mgr = NotificationManager::new(db, config, tx);

        mgr.handle_notify(1, "App", "", "ALERT", "", &[], 2, "", -1, true)
            .await;

        // Critical should broadcast even with DND on.
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, NotifyEvent::Added(_)));
    }

    #[tokio::test]
    async fn test_untrusted_critical_is_clamped_to_high() {
        use crate::dbus::server::Priority;
        let (mgr, _rx) = make_manager().await;

        // urgency 2 from an untrusted caller (may_set_critical = false) must be
        // stored as High, not the DND-piercing Critical (GAP-7).
        mgr.handle_notify(1, "Spoofer", "", "ALERT", "", &[], 2, "", -1, false)
            .await;
        let n = mgr.db.get_notification(1).await.unwrap().unwrap();
        assert_eq!(n.priority, Priority::High);

        // The same notification from a trusted caller keeps Critical.
        mgr.handle_notify(2, "powerd", "", "ALERT", "", &[], 2, "", -1, true)
            .await;
        let n = mgr.db.get_notification(2).await.unwrap().unwrap();
        assert_eq!(n.priority, Priority::Critical);
    }

    #[tokio::test]
    async fn test_handle_notify_rate_limited() {
        let (mgr, _rx) = make_manager().await;

        for i in 1..=10 {
            let id = mgr
                .handle_notify(i, "Spammy", "", "msg", "", &[], 1, "", -1, true)
                .await;
            assert_eq!(id, i);
        }

        // 11th should be rate-limited (returns 0).
        let id = mgr
            .handle_notify(11, "Spammy", "", "msg", "", &[], 1, "", -1, true)
            .await;
        assert_eq!(id, 0);
    }

    #[tokio::test]
    async fn test_handle_close() {
        let (mgr, mut rx) = make_manager().await;
        mgr.handle_notify(1, "App", "", "Hello", "", &[], 1, "", -1, true)
            .await;
        let _ = rx.try_recv(); // Drain the Added event.

        mgr.handle_close(1, CloseReason::Dismissed).await;

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, NotifyEvent::Closed { id: 1, .. }));

        // Should be dismissed in DB.
        assert_eq!(mgr.db.count_pending().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn test_activate_and_deactivate_focus() {
        let (mgr, mut rx) = make_manager().await;

        // Before focus: Slack notification passes.
        mgr.handle_notify(1, "Slack", "", "Msg", "hi", &[], 1, "", -1, true)
            .await;
        let _ = rx.try_recv(); // Drain.

        // Activate focus with Slack suppressed.
        mgr.activate_focus("proj-1".into(), vec!["Slack".into()])
            .await;

        mgr.handle_notify(2, "Slack", "", "Msg2", "hi", &[], 1, "", -1, true)
            .await;
        // Suppressed by focus — no broadcast.
        assert!(rx.try_recv().is_err());

        // A different app still passes.
        mgr.handle_notify(3, "Firefox", "", "Done", "", &[], 1, "", -1, true)
            .await;
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, NotifyEvent::Added(_)));

        // Deactivate -> Slack passes again.
        mgr.deactivate_focus().await;
        mgr.handle_notify(4, "Slack", "", "Msg3", "hi", &[], 1, "", -1, true)
            .await;
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, NotifyEvent::Added(_)));
    }

    #[tokio::test]
    async fn test_focus_critical_still_broadcasts() {
        let (mgr, mut rx) = make_manager().await;
        mgr.activate_focus("p".into(), vec!["Slack".into()]).await;

        // urgency=2 -> critical, bypasses focus suppression.
        mgr.handle_notify(1, "Slack", "", "ALERT", "", &[], 2, "", -1, true)
            .await;
        let event = rx.try_recv().unwrap();
        assert!(matches!(event, NotifyEvent::Added(_)));
    }

    #[tokio::test]
    async fn test_fullscreen_queues_and_flushes() {
        let (mgr, mut rx) = make_manager().await;
        mgr.set_fullscreen(true).await;

        mgr.handle_notify(1, "App", "", "Hello", "", &[], 1, "", -1, true)
            .await;

        // Should NOT broadcast (queued).
        assert!(rx.try_recv().is_err());

        // Exit fullscreen -> flush.
        mgr.set_fullscreen(false).await;

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, NotifyEvent::Added(_)));
    }

    #[tokio::test]
    async fn test_input_sanitization() {
        let (mgr, _rx) = make_manager().await;
        let long_name = "X".repeat(200);

        mgr.handle_notify(1, &long_name, "", "", "body", &[], 1, "", -1, true)
            .await;

        let n = mgr.db.get_notification(1).await.unwrap().unwrap();
        assert_eq!(n.app_name.len(), 50); // Truncated.
        assert_eq!(n.summary, n.app_name); // Empty summary -> app_name.
    }
}
