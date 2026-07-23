//! Client for the `org.arlen.JobViewServer1` D-Bus interface
//! (job-progress-surface.md).
//!
//! A producer (the file manager, forage, the model manager) reports a
//! long-running operation's progress so the shell's Activity/Jobs zone can
//! render it live. Behind the `client` feature so wire-type-only consumers (the
//! shell decoding `ServerMessage`) stay zbus-free.
//!
//! The `#[proxy]` macro generates two clients, so a producer picks by its
//! context: the async [`JobViewServerProxy`] (report fire-and-forget from an
//! async task so a report never blocks the operation - the right shape for an
//! interactive producer like the file manager, where the shell owns the
//! visibility threshold and instant ops must not pay a D-Bus round trip), or the
//! blocking `JobViewServerProxyBlocking` for a synchronous producer with no
//! async runtime (a CLI step, where a brief block is acceptable).
//!
//! ```no_run
//! # async fn ex(conn: &zbus::Connection) -> zbus::Result<()> {
//! use notification_proto::client::JobViewServerProxy;
//! let jobs = JobViewServerProxy::new(conn).await?;
//! // Register a determinate 200-file copy the shell may cancel.
//! let id = jobs
//!     .register("files", "Copy 200 files", "files", 200, true, true, false, "")
//!     .await?;
//! jobs.update(id, 50, 200, true).await?; // 50 of 200 done
//! jobs.finish(id).await?;
//! # Ok(())
//! # }
//! ```

use zbus::proxy;

/// The producer-side proxy for the job server the notification daemon hosts.
///
/// The method contract mirrors the daemon's `JobViewDbus`: `total` is ignored
/// when `determinate` is `false` (an indeterminate stretch), an empty
/// `egress_host`/`message` means "none", and `state` is one of `running` /
/// `paused` / `impeded` / `error-recoverable` / `error-fatal` / `done`. Every
/// call is best-effort from the producer's view - a job report must never break
/// the underlying operation, so a producer should ignore a returned error.
#[proxy(
    interface = "org.arlen.JobViewServer1",
    default_service = "org.arlen.JobViewServer1",
    default_path = "/org/arlen/JobViewServer"
)]
pub trait JobViewServer {
    /// Register a job and return its stable id (unique for the daemon's life).
    #[allow(clippy::too_many_arguments)]
    async fn register(
        &self,
        app_id: &str,
        title: &str,
        unit: &str,
        total: u64,
        determinate: bool,
        killable: bool,
        suspendable: bool,
        egress_host: &str,
    ) -> zbus::Result<u64>;

    /// Advance a job's processed amount (and optionally its total).
    async fn update(
        &self,
        id: u64,
        processed: u64,
        total: u64,
        determinate: bool,
    ) -> zbus::Result<bool>;

    /// Set a job's lifecycle state and its explanatory message.
    async fn set_state(&self, id: u64, state: &str, message: &str) -> zbus::Result<bool>;

    /// Remove a finished job from the live set.
    async fn finish(&self, id: u64) -> zbus::Result<bool>;
}
