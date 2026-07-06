//! Submission-CI lint for community-submitted permission profiles
//! (app-enrollment §E8). A profile is inert declarative data - it cannot run
//! code - so the review gate is two automated checks plus a human step. This
//! module is the first automated check: the **hard-deny lint**, the AWS IAM
//! `CheckAccessNotGranted` analogue. Any grant on the absolute hard-deny list can
//! NEVER auto-accept regardless of the app's declared category, so a profile
//! carrying one is auto-rejected. The second check (the subset-vs-category
//! baseline diff) is separate.

use std::path::Path;

use crate::PermissionProfile;

/// The hard-deny reasons a submitted profile carries, or empty if it clears the
/// lint. A non-empty result auto-rejects the submission. The list is fixed by the
/// plan: host-FS root, blanket home, another app's secrets or config, raw device
/// nodes, Knowledge Graph write, and global (system-wide) input capture. (The
/// audit socket and raw unmediated egress have no permission-profile field - an
/// app cannot request them through a profile - so they are structurally
/// unreachable here, not omitted.)
pub fn hard_deny_reasons(profile: &PermissionProfile) -> Vec<String> {
    let mut reasons = Vec::new();
    let app_id = profile.info.app_id.as_str();

    for path in &profile.filesystem.custom {
        if let Some(reason) = path_hard_deny_reason(path, app_id) {
            reasons.push(reason);
        }
    }
    if !profile.graph.write.is_empty() {
        reasons.push("grants Knowledge Graph write access".to_string());
    }
    if profile.input.register_global_bindings {
        reasons.push("grants global (system-wide) input capture".to_string());
    }
    reasons
}

/// Whether the submission clears the hard-deny lint (auto-mergeable on this axis).
pub fn passes_hard_deny(profile: &PermissionProfile) -> bool {
    hard_deny_reasons(profile).is_empty()
}

/// The hard-deny reason a single custom filesystem path carries, or `None` if it
/// is a benign app-local path. `app_id` lets the app reach its OWN config subtree
/// (`~/.config/<app_id>`) while any OTHER app's config is denied.
fn path_hard_deny_reason(path: &Path, app_id: &str) -> Option<String> {
    let raw = path.to_string_lossy();
    let s = raw.trim().trim_end_matches('/');

    // The host filesystem root, or the empty path that normalises to it.
    if s.is_empty() || s == "/" {
        return Some("grants the host filesystem root".to_string());
    }
    // Blanket access to the entire home directory.
    if matches!(s, "~" | "$HOME" | "${HOME}") {
        return Some("grants blanket access to the entire home directory".to_string());
    }
    // Raw device nodes.
    if s == "/dev" || s.starts_with("/dev/") {
        return Some(format!("grants a raw device node ({s})"));
    }
    // Another app's secrets.
    for secret in ["~/.ssh", "~/.gnupg", "~/.aws", "~/.pki", "~/.password-store"] {
        if s == secret || s.starts_with(&format!("{secret}/")) {
            return Some(format!("grants access to another app's secrets ({s})"));
        }
    }
    // System directories (the host-FS-root class).
    for sys in [
        "/etc", "/usr", "/bin", "/sbin", "/lib", "/boot", "/root", "/var", "/sys", "/proc", "/run",
    ] {
        if s == sys || s.starts_with(&format!("{sys}/")) {
            return Some(format!("grants a system directory ({s})"));
        }
    }
    // Another app's config under ~/.config (the app's own subtree is allowed).
    for prefix in ["~/.config/", "~/.local/share/"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let component = rest.split('/').next().unwrap_or("");
            if !component.is_empty() && component != app_id {
                return Some(format!("grants access to another app's data ({s})"));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml: &str) -> PermissionProfile {
        toml::from_str(toml).unwrap()
    }

    #[test]
    fn a_conservative_profile_clears_the_lint() {
        let p = parse("[info]\napp_id = \"com.x\"\n[filesystem]\ndocuments = true\n[network]\nallow_all = true\n");
        assert!(passes_hard_deny(&p), "{:?}", hard_deny_reasons(&p));
    }

    #[test]
    fn dangerous_custom_paths_are_denied() {
        for path in [
            "/",
            "~",
            "/dev/sda",
            "~/.ssh",
            "~/.gnupg/secring.gpg",
            "/etc/shadow",
            "~/.config/org.other.App",
        ] {
            let p = parse(&format!(
                "[info]\napp_id = \"com.x\"\n[filesystem]\ncustom = [\"{path}\"]\n"
            ));
            assert!(!passes_hard_deny(&p), "{path} should be hard-denied");
        }
    }

    #[test]
    fn an_app_may_reach_its_own_config_subtree() {
        let p = parse("[info]\napp_id = \"com.x\"\n[filesystem]\ncustom = [\"~/.config/com.x\"]\n");
        assert!(passes_hard_deny(&p), "{:?}", hard_deny_reasons(&p));
    }

    #[test]
    fn kg_write_and_global_input_are_denied() {
        let p = parse("[info]\napp_id = \"com.x\"\n[graph]\nwrite = [\"system.File\"]\n");
        assert!(!passes_hard_deny(&p));
        let p = parse("[info]\napp_id = \"com.x\"\n[input]\nregister_global_bindings = true\n");
        assert!(!passes_hard_deny(&p));
    }
}
