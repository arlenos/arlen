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

use arlen_permissions::revoke::{RevokeInitiator, RevokeReach, RevokedReach};
use os_sdk::graph::{GrantView, UnixGraphClient};

/// The knowledge daemon's read socket. `ARLEN_DAEMON_SOCKET` overrides it (dev and
/// per-user socket layouts); the default is the system path the packaged daemon
/// binds.
fn knowledge_socket() -> String {
    std::env::var("ARLEN_DAEMON_SOCKET").unwrap_or_else(|_| "/run/arlen/knowledge.sock".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_default_and_override() {
        // The default is the packaged system path; the env var wins for dev / a
        // per-user socket layout.
        std::env::remove_var("ARLEN_DAEMON_SOCKET");
        assert_eq!(knowledge_socket(), "/run/arlen/knowledge.sock");
        std::env::set_var("ARLEN_DAEMON_SOCKET", "/run/user/1000/arlen/knowledge.sock");
        assert_eq!(knowledge_socket(), "/run/user/1000/arlen/knowledge.sock");
        std::env::remove_var("ARLEN_DAEMON_SOCKET");
    }

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
