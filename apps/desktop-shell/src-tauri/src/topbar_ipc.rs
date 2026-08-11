// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! The route Settings needs to read the live top-bar inventory.
//!
//! `topbar_items` is implemented and registered - in the desktop-shell. A Tauri
//! command only exists inside the binary that registers it, so Settings invoking
//! it by name resolves to nothing and its arrangement panel comes up empty with
//! an error. **The producer was never missing; the route was.** That is a
//! different job from writing a command, and mistaking one for the other is why
//! it sat on the missing-command list for weeks looking like unwritten work.
//!
//! The inventory is live state - which applets exist and which tray items are
//! currently registered - so it cannot come from a config file. It has to come
//! from the running shell, which is what this broker is.
//!
//! Shaped like `search_ipc` next door, minus everything a read-only inventory
//! does not need: **no request body at all.** A caller connects, is
//! authenticated, and is written the JSON array. There is nothing to parse from
//! the peer, so there is no parser to get wrong.
//!
//! **Admission is an allowlist, not just same-uid.** The inventory names every
//! tray item the user is running, which is a list of the applications they have
//! open - modest, and still nobody's business but the arrangement panel's. The
//! ids are the ones the rest of the tree already keys on (`config-broker`'s
//! `ADMITTED_WRITERS`, `consent-broker`'s `CONTROL_ADMITTED`).

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use arlen_permissions::ConnectionAuth;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncWriteExt;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;
use tokio::time::timeout;

use crate::sni::SniItems;

const SOCKET_NAME: &str = "topbar.sock";

/// Callers admitted to read the inventory. Settings is the arrangement panel;
/// the release id and the dev-build id are both here because the resolver
/// answers `settings` for the installed binary and `dev.arlen.settings` for a
/// cargo-run one, and a broker that works only in a release build is a broker
/// nobody can develop against.
const ADMITTED: &[&str] = &["settings", "dev.arlen.settings"];

/// Cap on simultaneous connections. The response is small and the exchange is
/// one write, so this only exists to keep a flood from spawning tasks without
/// bound: excess accepts are dropped rather than queued.
const MAX_CONCURRENT_CONNS: usize = 8;

/// How long the whole exchange may take. There is no request to wait for, so
/// this bounds a peer that accepts the connection and then never reads.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

fn semaphore() -> Arc<Semaphore> {
    static SEM: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SEM.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_CONNS)))
        .clone()
}

/// Where the broker binds, and where a client dials.
pub fn socket_path() -> Result<PathBuf, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| "XDG_RUNTIME_DIR not set".to_string())?;
    let mut p = PathBuf::from(runtime);
    p.push("arlen");
    p.push(SOCKET_NAME);
    Ok(p)
}

/// Whether a resolved caller may read the inventory.
///
/// Split out from the connection path so the rule is testable without a socket:
/// the admission is the whole of the policy here, and a policy only exercised
/// through a live peer is one nobody checks.
fn admitted(app_id: &str) -> bool {
    ADMITTED.contains(&app_id)
}

/// Bind the socket and spawn the accept loop.
pub fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        match run(app).await {
            Ok(()) => log::info!("topbar_ipc: shut down cleanly"),
            Err(e) => log::error!("topbar_ipc: server exited: {e}"),
        }
    });
}

async fn run(app: AppHandle) -> Result<(), String> {
    let path = socket_path().map_err(|e| format!("derive socket path: {e}"))?;
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let listener = UnixListener::bind(&path).map_err(|e| format!("bind {}: {e}", path.display()))?;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    log::info!("topbar_ipc: listening on {}", path.display());

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let Ok(permit) = semaphore().try_acquire_owned() else {
                    log::warn!("topbar_ipc: connection cap reached, dropping accept");
                    drop(stream);
                    continue;
                };
                let app = app.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = serve(stream, app).await {
                        log::warn!("topbar_ipc: connection ended: {e}");
                    }
                });
            }
            Err(e) => {
                log::warn!("topbar_ipc: accept failed: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Authenticate the peer and write it the inventory. One write, then close.
async fn serve(stream: UnixStream, app: AppHandle) -> Result<(), String> {
    let caller_uid = unsafe { libc::getuid() };
    let auth = ConnectionAuth::extract_from(&stream, caller_uid).map_err(|e| format!("auth: {e}"))?;
    if !admitted(auth.app_id()) {
        log::info!("topbar_ipc: refusing {} (not admitted)", auth.app_id());
        return Err(format!("{} is not admitted", auth.app_id()));
    }

    let sni = app.state::<SniItems>();
    let items = crate::topbar::topbar_items(sni)?;
    let body = serde_json::to_vec(&items).map_err(|e| format!("encode: {e}"))?;

    let mut stream = stream;
    timeout(WRITE_TIMEOUT, async {
        stream.write_all(&body).await.map_err(|e| format!("write: {e}"))?;
        stream.shutdown().await.map_err(|e| format!("shutdown: {e}"))
    })
    .await
    .map_err(|_| "write timed out".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole policy, and the reason it is a function rather than an inline
    /// `contains`: a broker that admits everyone same-uid would hand any app the
    /// list of tray items the user is running.
    #[test]
    fn only_settings_is_admitted() {
        assert!(admitted("settings"));
        assert!(admitted("dev.arlen.settings"));
        assert!(!admitted("dev.arlen.files"));
        assert!(!admitted("unknown"));
        assert!(!admitted(""));
    }

    /// The path both ends have to agree on. A client dialling elsewhere gets a
    /// missing socket, which reads as "the shell is not running" - the failure
    /// this broker exists to stop being ambiguous.
    #[test]
    fn the_socket_sits_beside_the_other_shell_brokers() {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1000");
        let p = socket_path().expect("a runtime dir is set");
        assert_eq!(p, PathBuf::from("/run/user/1000/arlen/topbar.sock"));
    }
}
