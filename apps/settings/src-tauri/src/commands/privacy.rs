//! The App-access capability browser's backend bridge.
//!
//! arlen-ui's privacy page (`apps/settings/src/routes/privacy`) renders the
//! system-wide capability grant list and per-reach revoke, the home of revoke the
//! harness defers here. These commands are the Settings-side half: they connect to
//! the knowledge daemon's read socket as the kernel-attested `settings` principal,
//! which the daemon admits for the whole-system grant browse (`access_grants`) and
//! for narrowing-only revokes (`revoke`, living-capability-graph.md §6). No AI
//! scope is involved (the harness keeps its own AI-principal copy in `ai_manage`);
//! this is the user's management surface over every app's capabilities.

use arlen_permissions::revoke::{RestoreReach, RevokeInitiator, RevokeReach, RevokedReach};
use os_sdk::graph::{GrantView, UnixGraphClient};

/// The knowledge daemon's read socket, through the shared resolver.
///
/// This read `ARLEN_DAEMON_SOCKET` and then fell straight to `/run/arlen`, which
/// is two misses at once: the socket answers to two env names and the daemon is a
/// user service, so on a booted session it binds under `$XDG_RUNTIME_DIR/arlen/`
/// and `arlen-session` pins neither name. The App-access panel and its revoke
/// would have dialled a path nothing binds. `knowledge_socket_path` is where both
/// facts live once - it was written after the same miss cost six resolvers.
fn knowledge_socket() -> String {
    os_sdk::runtime::knowledge_socket_path()
        .to_string_lossy()
        .into_owned()
}

/// The whole-system capability grant list for the App-access panel.
///
/// Calls the daemon's `access_grants` op as the `settings` principal, so it returns
/// every app's grants (not the caller-scoped slice an ordinary app receives). Each
/// [`GrantView`] carries the full declared ceiling and lifecycle flags the panel
/// renders (the frontend store mirrors the shape 1:1). Returns an error string on a
/// transport or daemon failure so the panel can show a degraded state rather than
/// silently rendering nothing.
#[tauri::command]
pub async fn access_grants() -> Result<Vec<GrantView>, String> {
    UnixGraphClient::new(knowledge_socket())
        .access_grants()
        .await
        .map_err(|e| e.to_string())
}

/// Narrow one reach of a target app's capability profile (the panel's per-scope
/// revoke).
///
/// `reach` is the JSON for the closed [`RevokedReach`] enum, e.g.
/// `{"Read":{"entity_pattern":"system.File"}}` (a read+write line issues one call
/// per side). The daemon admits only the `settings` principal, refuses a
/// system-tier target, and applies the narrowing through the strict-subset gate, so
/// the closed request cannot express a widening. Returns the outcome wire token
/// (`OK: revoked` / `no-change` / `not-narrowing` / `not-found`) for the panel to
/// surface; a malformed `reach` or a daemon error returns an error string.
#[tauri::command]
pub async fn revoke_reach(target_app_id: String, reach: String) -> Result<String, String> {
    let reach: RevokedReach =
        serde_json::from_str(&reach).map_err(|e| format!("invalid reach: {e}"))?;
    let request = RevokeReach {
        target_app_id,
        reach,
        initiator: RevokeInitiator::User,
    };
    let outcome = UnixGraphClient::new(knowledge_socket())
        .revoke(&request)
        .await
        .map_err(|e| e.to_string())?;
    Ok(outcome.wire_token().to_string())
}

/// Re-widen one reach the user previously revoked (the panel's per-scope restore),
/// the reverse of [`revoke_reach`] and the one authority-growth path.
///
/// `reach` is the JSON for the closed [`RevokedReach`] enum naming the reach to
/// re-add. The daemon admits only the `settings` principal, refuses a system-tier
/// target, and bounds the reach to a recorded removal in the durable audit ledger,
/// so a restore can only un-do a specific prior revoke, never grant fresh authority.
/// Returns the outcome wire token (`OK: restored` / `no-change` / `not-permitted` /
/// `not-found`); a malformed `reach` or a daemon error returns an error string.
#[tauri::command]
pub async fn restore_reach(target_app_id: String, reach: String) -> Result<String, String> {
    let reach: RevokedReach =
        serde_json::from_str(&reach).map_err(|e| format!("invalid reach: {e}"))?;
    let request = RestoreReach {
        target_app_id,
        reach,
        initiator: RevokeInitiator::User,
    };
    let outcome = UnixGraphClient::new(knowledge_socket())
        .restore(&request)
        .await
        .map_err(|e| e.to_string())?;
    Ok(outcome.wire_token().to_string())
}

/// Release a remembered CONSENT grant by its revocation handle (the "Access to
/// ~/Documents" consent lines in the App-access panel), distinct from the
/// profile-scope [`revoke_reach`]: that narrows a declared capability, this
/// releases a consent record. Routed through the consent broker's control socket,
/// which admits `settings` for the grant-management ops (list + revoke) but not
/// for answering prompts. Returns `"revoked"`, or `"no-change"` for an unknown or
/// already-revoked handle. The client transport is synchronous, so it runs on the
/// blocking pool.
#[tauri::command]
pub async fn revoke_consent(grant_id: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        arlen_consent_broker::ControlClient::at_default_path()?.revoke_grant(&grant_id)
    })
    .await
    .map_err(|e| format!("join: {e}"))?
    .map(|ok| if ok { "revoked" } else { "no-change" }.to_string())
    .map_err(|e| format!("revoke_consent: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // `socket_default_and_override` lived here and was deleted on 9 September. It
    // asserted that `knowledge_socket()` answers `/run/arlen/knowledge.sock` when
    // no env var pins one - which is only true where `XDG_RUNTIME_DIR` is unset.
    // It is unset in a CI container and set on every desktop, so the test passed
    // in CI and failed for anyone who ran the suite locally, which is the worst
    // way round: the machine nobody watches says yes and the machine somebody is
    // working on says no.
    //
    // It was also testing another crate's rule through this one. The resolution
    // order lives in `os_sdk::runtime::resolve`, which is pure and has five tests
    // covering every branch of it - including the two this one confused. Nothing
    // here owns that behaviour, so nothing here should assert it.

    #[test]
    fn revoke_reach_parses_the_closed_enum() {
        // The panel's per-line JSON parses into the closed RevokedReach; a garbage
        // string is rejected before any socket call.
        let read: RevokedReach =
            serde_json::from_str(r#"{"Read":{"entity_pattern":"system.File"}}"#).unwrap();
        assert!(matches!(read, RevokedReach::Read { .. }));
        assert!(serde_json::from_str::<RevokedReach>("not json").is_err());
    }
}
