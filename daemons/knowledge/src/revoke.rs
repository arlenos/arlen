//! Where a revoke is applied, and who may be revoked.
//!
//! The mechanism (the closed request vocabulary, the strict-subset gate, the
//! format-preserving edit, the removal ledger) is
//! `arlen_permissions::revoke_apply`, shared with the root permission helper.
//! What stays here is the daemon's own part: which principals a revoke may
//! name, and which tier a given app's revoke has to reach.

use std::path::Path;

use crate::quota::{AppTier, QuotaConfig};

// The wire-contract types and the mechanism (the scope summary, the narrowing
// gate, the format-preserving edit, the removal ledger) live in
// `arlen-permissions`, because two processes apply a revoke: this daemon for a
// profile in the user tier and the root permission helper for one in the system
// tier. Re-exported so `crate::revoke::…` resolves as before.
pub use arlen_permissions::revoke::{
    RestoreOutcome, RestoreReach, RevokeInitiator, RevokeOutcome, RevokeReach, RevokedReach,
};
pub use arlen_permissions::revoke_apply::*;

/// Whether the user tier may revoke this app's reach (§6.2): apps yes, system
/// principals no (their authority is managed by the system, not revocable
/// through this path). This keys on the QUOTA tier, which names the core
/// principals; it says nothing about which directory a profile is filed in.
pub fn tier_allows_revoke(app_id: &str) -> bool {
    QuotaConfig::arlen_default().tier_for_app(app_id) != AppTier::System
}

/// Where a revoke or a restore of `app_id` has to be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileTier {
    /// A root-owned profile under `/var/lib/arlen/permissions` decides this
    /// app's authority, so only the permission helper can change it.
    System,
    /// The app's profile is the user's own file, which this daemon edits.
    User(std::path::PathBuf),
}

/// Which tier decides `app_id`'s authority, by the loader's own rule: a
/// system-tier profile wins outright when it exists
/// (`arlen_permissions::load_profile`).
///
/// A revoke used to be written to the user's file whichever tier the app read
/// from. Every first-party app on the image reads the system tier, so for those
/// a revoke reported `not-found` at best and, where a stale user file existed,
/// reported `revoked` while the app kept its access.
pub fn tier_of(app_id: &str) -> Result<ProfileTier, arlen_permissions::PermissionError> {
    let user = arlen_permissions::profile_path(app_id)?;
    Ok(tier_between(&arlen_permissions::system_permissions_dir(), app_id, user))
}

/// [`tier_of`] over a named system directory and user file.
fn tier_between(system_dir: &Path, app_id: &str, user: std::path::PathBuf) -> ProfileTier {
    if system_dir.join(format!("{app_id}.toml")).exists() {
        ProfileTier::System
    } else {
        ProfileTier::User(user)
    }
}

/// Ask the root permission helper to change one reach of a system-tier
/// profile. `method` is `NarrowProfile` or `RestoreReach`; the answer is the
/// outcome's wire token, the same string the user-tier path produces.
///
/// The helper proves a removal shrinks the profile and gives back only what it
/// recorded as removed, so what this daemon can cause through it is bounded
/// there and not here.
pub async fn change_system_tier(
    method: &'static str,
    app_id: &str,
    reach: &RevokedReach,
) -> Result<String, String> {
    let reach_json = serde_json::to_string(reach).map_err(|e| e.to_string())?;
    // SAFETY: getuid never fails.
    let uid = unsafe { libc::getuid() };
    let conn = zbus::Connection::system().await.map_err(|e| format!("system bus: {e}"))?;
    let proxy = zbus::Proxy::new(
        &conn,
        "org.arlen.PermissionHelper1",
        "/org/arlen/PermissionHelper1",
        "org.arlen.PermissionHelper1",
    )
    .await
    .map_err(|e| format!("permission helper: {e}"))?;
    let (ok, answer): (bool, String) = proxy
        .call(method, &(app_id, uid, reach_json.as_str()))
        .await
        .map_err(|e| format!("permission helper: {e}"))?;
    if ok {
        Ok(answer)
    } else {
        Err(answer)
    }
}

#[cfg(test)]
mod tier_tests {
    use super::*;

    #[test]
    fn the_tier_that_decides_is_the_one_a_revoke_goes_to() {
        let system = tempfile::TempDir::new().unwrap();
        let user = tempfile::TempDir::new().unwrap();
        let mine = |id: &str| user.path().join(format!("{id}.toml"));
        std::fs::write(mine("com.both"), "").unwrap();
        std::fs::write(system.path().join("com.both.toml"), "").unwrap();
        std::fs::write(mine("com.mine"), "").unwrap();

        // A user file beside a system one decides nothing: the loader never
        // reads it, so a revoke written there would be for show.
        assert_eq!(tier_between(system.path(), "com.both", mine("com.both")), ProfileTier::System);
        assert_eq!(
            tier_between(system.path(), "com.mine", mine("com.mine")),
            ProfileTier::User(mine("com.mine"))
        );
    }
}
