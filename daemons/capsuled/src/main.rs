//! The `capsuled` daemon: serve same-machine Context Capsule reads
//! (context-capsule.md §6).
//!
//! Opens the persisted capsule signing key, the frozen-slice store and the durable
//! revoke/op-count ledger, then serves the SO_PEERCRED Unix socket. A reader
//! presents a signed grant; a valid, unrevoked, unexpired, in-budget grant gets
//! the frozen slice, every read audited fail-closed. Minting (which materializes a
//! slice and registers a grant) is the human-gated surface (CC-R6); this daemon is
//! the serve + revoke-enforcement half.

use std::path::Path;
use std::sync::Arc;

use arlen_forage_store::Store;
use audit_proto::LedgerAuditSink;
use capsuled::key::{capsule_key_path, CapsuleSigningKey};
use capsuled::revocation::RevocationFile;
use capsuled::server::{run, socket_path, ServeContext};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let key_path = capsule_key_path().ok_or("no XDG_STATE_HOME or HOME for the capsule key")?;
    let key = CapsuleSigningKey::load_or_create(&key_path)?;
    let verifying_key = key.verifying_key();

    // The capsule state dir is the signing key's parent (arlen/capsule/): the slice
    // store and the revoke ledger live alongside it.
    let state_dir = key_path
        .parent()
        .ok_or("capsule key path has no parent")?
        .to_path_buf();
    let store = Store::open(state_dir.join("store"))?;
    let ledger = RevocationFile::open(&state_dir)?;
    let audit = LedgerAuditSink::at_default_socket();

    let sock = socket_path().ok_or("no XDG_RUNTIME_DIR for the capsule socket")?;
    // Pre-create the socket dir before the fence so its write grant is
    // expressible (an absent grant path is skipped fail-safe).
    if let Some(parent) = sock.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Self-confine (Tier-A #2): read everywhere, write only under the capsule
    // state dir (the signing key, the frozen-slice store, the revoke ledger) and
    // the socket dir. All the genesis writes above (key/store/ledger init) ran
    // before this; the only post-fence writes are the serve-time revoke-ledger
    // updates + op-count decrements, all under state_dir. The signing key never
    // leaves the process, so a compromised capsuled cannot exfiltrate it by
    // writing it elsewhere. Fenced on the main thread BEFORE the runtime is built
    // so every tokio worker inherits the Landlock domain. The daemon spawns no
    // child, so there is no inherited-domain concern.
    apply_fence(&state_dir, &sock);

    let ctx = ServeContext {
        verifying_key,
        ledger: Arc::new(ledger),
        store: Arc::new(store),
        audit: Arc::new(audit),
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        tracing::info!(socket = %sock.display(), "capsule daemon listening");
        tokio::select! {
            r = run(&sock, ctx) => { r?; }
            _ = shutdown_signal() => { tracing::info!("capsule daemon shutting down"); }
        }
        // Best-effort socket cleanup on a clean exit.
        let _ = std::fs::remove_file(&sock);
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

/// Install the Landlock write-fence over the capsule state dir and the socket
/// dir. Defense-in-depth: a kernel that cannot enforce it leaves the daemon
/// exactly as safe as no fence, so by default a non-enforcing kernel or a
/// ruleset error is logged and the daemon continues. A hardened deployment that
/// wants the confinement guaranteed sets `ARLEN_CAPSULE_REQUIRE_FENCE=1`, making
/// a non-enforcing kernel a fatal startup error.
fn apply_fence(state_dir: &Path, sock: &Path) {
    use arlen_landlock_fence::{fence_writes, FenceOutcome};
    let require = std::env::var_os("ARLEN_CAPSULE_REQUIRE_FENCE").is_some_and(|v| v == "1");
    let mut writable: Vec<&Path> = vec![state_dir];
    if let Some(p) = sock.parent() {
        writable.push(p);
    }
    let degraded = match fence_writes(&writable) {
        Ok(FenceOutcome::Enforced) => {
            tracing::info!("landlock write-fence enforced (write-confined to state + socket dirs)");
            None
        }
        Ok(FenceOutcome::NotEnforced) => Some("landlock not enforced by this kernel".to_string()),
        Err(e) => Some(format!("landlock fence not applied: {e}")),
    };
    if let Some(reason) = degraded {
        if require {
            tracing::error!(
                "ARLEN_CAPSULE_REQUIRE_FENCE=1 but the fence is not active ({reason}); refusing to run unconfined"
            );
            std::process::exit(1);
        }
        tracing::warn!("{reason}; running unconfined (no worse than no fence)");
    }
}

/// Resolve when the daemon should shut down: SIGINT (ctrl-c) or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
}
