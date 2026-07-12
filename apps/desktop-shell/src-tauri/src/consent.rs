//! Tauri commands wrapping the consent broker's control client, so the shell's
//! consent dialog can fetch the front pending request and submit the user's
//! decision. The broker (daemons/consent-broker) attests the shell peer over
//! SO_PEERCRED and owns the queue, severity-tier resolution, grant persistence
//! and audit; these commands are the thin transport the dialog store
//! (`lib/stores/consent.ts`) drives once its fixture is swapped for the live path.
//!
//! `ControlClient` is a synchronous one-shot-per-connection client, so each call
//! runs on a blocking thread to keep the async runtime free.

use arlen_consent_broker::control::PendingView;
use arlen_consent_broker::control_client::ControlClient;
use arlen_consent_broker::ConsentOutcome;

/// Restore the process's dumpable attribute before connecting to the consent
/// broker's CONTROL socket.
///
/// The broker attests the shell peer by resolving its identity from
/// `/proc/<pid>/exe` (SO_PEERCRED pid -> path_to_app_id). A same-uid, non-root
/// reader (the broker) can only readlink another process's `exe` when that process
/// is *dumpable*; WebKitGTK clears the dumpable bit (`PR_SET_DUMPABLE=0`) when it
/// initialises the webview sandbox, so the shell - an ordinary unhardened app the
/// identity model expects to be dumpable (see `arlen-dogfood.service`) - would
/// otherwise be unresolvable and every consent fetch/resolve refused with
/// `cannot read exe path: Permission denied`. Restoring the bit puts the shell back
/// in the "ordinary dumpable app caller" state the model assumes; it opens no new
/// exposure, since same-uid processes are already one trust domain (the same-uid
/// boundary is the F3/separate-uid concern, not this).
fn ensure_dumpable() {
    use std::sync::Once;
    static DIAG: Once = Once::new();
    DIAG.call_once(|| {
        // One-shot diagnostic (lands in the journal -> serial for boot-verify): the
        // pre-restore value; `Dumpable: 0` confirms the WebKit clear is the cause.
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            if let Some(line) = status.lines().find(|l| l.starts_with("Dumpable:")) {
                eprintln!("consent: shell {} before dumpable-restore", line.trim());
            }
        }
    });
    // SAFETY: prctl(PR_SET_DUMPABLE, 1) is a documented syscall that only sets the
    // calling process's dumpable attribute; no memory or fd is touched.
    unsafe {
        libc::prctl(libc::PR_SET_DUMPABLE, 1);
    }
}

/// Fetch the front pending consent request to render, or `None` when nothing is
/// pending. Wraps the broker's `ControlClient::fetch`.
#[tauri::command]
pub async fn consent_fetch() -> Result<Option<PendingView>, String> {
    tokio::task::spawn_blocking(|| {
        ensure_dumpable();
        ControlClient::at_default_path().fetch()
    })
    .await
    .map_err(|e| format!("consent fetch task failed: {e}"))?
    .map_err(|e| format!("consent fetch: {e}"))
}

/// Submit the user's decision for a pending request. Returns `false` if the id was
/// unknown or already resolved. Wraps the broker's `ControlClient::resolve`, which
/// removes it from the queue, replies to the waiting requester and persists a
/// grant for an always-allow.
#[tauri::command]
pub async fn consent_resolve(id: u64, outcome: ConsentOutcome) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || {
        ensure_dumpable();
        ControlClient::at_default_path().resolve(id, outcome)
    })
    .await
    .map_err(|e| format!("consent resolve task failed: {e}"))?
    .map_err(|e| format!("consent resolve: {e}"))
}
