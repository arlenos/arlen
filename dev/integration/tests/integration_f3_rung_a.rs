//! F3 Rung A IPC-boundary proof: the orphan `permission-helper` write is no longer
//! an orphan (identity-spoof-mitigation.md §4, AUTH-CANONICAL.md §2).
//!
//! The full live path is: an apt enroll hook fires `installd.EnrollSystemApp`, which
//! generates the profile and calls `org.arlen.PermissionHelper1.WriteProfile` over
//! the system bus, which writes it root-owned under
//! `/var/lib/arlen/permissions/{uid}/{app_id}.toml`; thereafter
//! `arlen_permissions::load_profile` prefers that root-owned profile over any
//! user-tier `~/.config` overlay, so a same-uid process cannot widen a
//! system-installed app's grants. That live hop needs the system bus, a root helper
//! and the `arlen-installd` caller exe, so it runs only in the integration stage,
//! by hand:
//!
//!   sudo arlen-permission-helper &                 # system bus, root-owned writes
//!   busctl --user call org.arlen.InstallDaemon1 /org/arlen/InstallDaemon1 \
//!       org.arlen.InstallDaemon1 EnrollSystemApp ss <app_id> <manifest_path>
//!
//! What this test pins deterministically (no bus, no root) is the security-bearing
//! end-state contract the wiring must produce: through the public, env-driven
//! `load_profile`, a system-tier profile WINS over a wider user-tier overlay. The
//! `#[ignore]` marks it as an F3-boundary test exercised in the integration stage,
//! matching the repo convention; it manipulates process-global env vars, so it runs
//! alone, not in the parallel default run.

use std::fs;

/// Write the two tiers, point the loader at both via its dev/test env overrides, and
/// assert the root-owned system tier wins and the user overlay cannot widen it.
#[test]
#[ignore = "F3 integration-stage boundary test; manipulates global env, run alone"]
fn the_system_tier_profile_wins_over_a_wider_user_overlay() {
    let tmp = tempfile::tempdir().unwrap();
    let sys_dir = tmp.path().join("system");
    let user_dir = tmp.path().join("user");
    fs::create_dir_all(&sys_dir).unwrap();
    fs::create_dir_all(&user_dir).unwrap();

    let app_id = "com.example.notes";

    // The root-owned system-tier profile (what the permission-helper writes): a
    // tight grant. The override resolves directly to `<dir>/{app_id}.toml`.
    fs::write(
        sys_dir.join(format!("{app_id}.toml")),
        "[info]\napp_id = \"com.example.notes\"\ntier = \"system\"\n\
         [graph]\nread = [\"system.File.path\"]\n",
    )
    .unwrap();

    // A same-uid user overlay that tries to widen the grant to everything.
    fs::write(
        user_dir.join(format!("{app_id}.toml")),
        "[info]\napp_id = \"com.example.notes\"\ntier = \"third-party\"\n\
         [graph]\nread = [\"system.File\", \"system.Session\", \"shared.Person\"]\n",
    )
    .unwrap();

    std::env::set_var("ARLEN_SYSTEM_PERMISSIONS_DIR", &sys_dir);
    std::env::set_var("ARLEN_PERMISSIONS_DIR", &user_dir);

    let profile = arlen_permissions::load_profile(app_id).expect("load_profile");

    // The system tier won: exactly the one tight read scope, none of the overlay's.
    assert_eq!(profile.info.tier, arlen_permissions::AppTier::System);
    assert_eq!(
        profile.graph.read,
        vec!["system.File.path".to_string()],
        "the wider user overlay must not widen the root-owned grant"
    );

    std::env::remove_var("ARLEN_SYSTEM_PERMISSIONS_DIR");
    std::env::remove_var("ARLEN_PERMISSIONS_DIR");
}
