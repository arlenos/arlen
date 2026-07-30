//! What revoking an extension actually does.
//!
//! "Revoke" is one word over three different operations. An app's authority
//! lives in its permission profile and is narrowed there; a module's lives in
//! the consent store as grants attributed to it; a bridge's is a delegated
//! namespace on the ingest profile. Hiding that behind one abstraction would
//! produce a button that cannot say which of the three it just ran, and a user
//! who cannot tell whether it worked.
//!
//! So this produces a PLAN, not an effect. The surface shows it, the user
//! confirms it, and whichever service owns each step runs it. Planning stays
//! pure and this crate stays free of daemon dependencies.
//!
//! **The residue is the load-bearing half.** Every one of these removes future
//! authority and none of them undoes the past: a narrowed profile does not
//! unread what the app already read, a dropped consent grant does not undo what
//! the module did with it, and removing a bridge's namespace leaves everything
//! it already wrote sitting in the knowledge graph. A revoke button that
//! implies otherwise is worse than no button, because the user stops looking.

use serde::{Deserialize, Serialize};

use arlen_permissions::revoke::RevokedReach;

use crate::{Extension, ExtensionKind};

/// One concrete operation, named by the service that owns it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "step")]
pub enum RevokeStep {
    /// Narrow the app's permission profile. Narrowing only: the knowledge
    /// daemon's revoke op refuses anything that is not a strict subset, so
    /// this can never widen and never needs to be trusted not to.
    NarrowProfile {
        /// The app whose profile is narrowed.
        app_id: String,
        /// The capability labels being given up.
        capabilities: Vec<String>,
    },
    /// Drop the consent grants recorded for this module. They are attributed
    /// to the module rather than to modulesd, so this removes one extension's
    /// authority without touching any other's.
    DropConsentGrants {
        /// The module the grants are recorded against.
        module_id: String,
    },
    /// Remove the bridge's delegated namespace, after which it can write
    /// nothing at all - a bridge holds exactly one namespace, so this is total
    /// rather than partial.
    RemoveNamespaceGrant {
        /// The namespace the bridge held.
        namespace: String,
    },
}

/// What revoking would do, and what it would leave behind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevokePlan {
    /// The operations, in the order they should run. Empty when the extension
    /// holds nothing - which is a real answer, not a failure.
    pub steps: Vec<RevokeStep>,
    /// What this does NOT undo, in the user's words. Never empty when there is
    /// a step, because no step reaches backwards.
    pub residue: Vec<String>,
}

impl RevokePlan {
    /// Whether there is anything to revoke.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Plan the revocation of `extension`.
///
/// An extension with no capabilities yields no steps AND no residue: nothing
/// was granted, so nothing is given up and nothing was done with it. Offering
/// a confirm dialog there would ask the user to approve the empty set, the same
/// reason the module consent gate skips a manifest that declares nothing.
pub fn plan(extension: &Extension) -> RevokePlan {
    if extension.capabilities.is_empty() {
        return RevokePlan {
            steps: Vec::new(),
            residue: Vec::new(),
        };
    }
    let (steps, residue) = match extension.kind {
        ExtensionKind::App => (
            vec![RevokeStep::NarrowProfile {
                app_id: extension.id.clone(),
                capabilities: extension.capabilities.clone(),
            }],
            vec![format!(
                "{} keeps anything it already read or wrote; this only stops it doing more.",
                extension.name
            )],
        ),
        ExtensionKind::Module => (
            vec![RevokeStep::DropConsentGrants {
                module_id: extension.id.clone(),
            }],
            vec![format!(
                "{} keeps anything it already did; this only stops it doing more.",
                extension.name
            )],
        ),
        ExtensionKind::Bridge => {
            let namespace = namespace_of(extension);
            (
                vec![RevokeStep::RemoveNamespaceGrant {
                    namespace: namespace.clone(),
                }],
                vec![
                    format!("{} can write nothing further.", extension.name),
                    // The residue a user is most likely to assume away, and the
                    // one that is a whole design question (BR-6) rather than an
                    // oversight: efficiently deleting every node a source wrote
                    // is unsolved, so this says so instead of implying the data
                    // went with the grant.
                    format!(
                        "Everything {namespace} already ingested stays in your knowledge graph. \
                         Removing it is a separate step that does not exist yet."
                    ),
                ],
            )
        }
    };
    RevokePlan { steps, residue }
}

/// A bridge's namespace, recovered from its single `write:` label.
///
/// The label is the authority (`crate::bridge::bridge_labels` emits exactly
/// one), so reading it back is reading the same fact rather than re-deriving
/// it from the id and risking the two disagreeing.
fn namespace_of(extension: &Extension) -> String {
    extension
        .capabilities
        .iter()
        .find_map(|c| c.strip_prefix("write:"))
        .unwrap_or(&extension.id)
        .to_string()
}

/// The concrete reaches that giving up `labels` means for this profile.
///
/// Resolved against the profile at the moment of revoking rather than baked
/// into the plan, and that is a correctness point rather than a style one: the
/// plan is built from an inventory read that may be seconds or minutes old, and
/// an app whose grants changed since would otherwise have a stale set revoked -
/// either missing what it gained or refusing over what it already gave up.
///
/// A label expands to every grant under it, because the label IS the coarse
/// answer: a user giving up "network" means all of it, not the first domain.
/// `allow_all` yields no domain reaches, matching the revoke gate, which
/// refuses removing a list entry while the blanket flag makes the list moot -
/// so that combination is reported by [`unrevocable`] instead of silently
/// producing steps that would be refused.
pub fn resolve_reaches(
    profile: &arlen_permissions::PermissionProfile,
    labels: &[String],
) -> Vec<RevokedReach> {
    let mut out = Vec::new();
    for label in labels {
        match label.as_str() {
            "network" if !profile.network.allow_all => {
                for domain in &profile.network.allowed_domains {
                    out.push(RevokedReach::NetworkDomain {
                        domain: domain.clone(),
                    });
                }
            }
            "filesystem" => {
                let fs = &profile.filesystem;
                for (flag, on) in [
                    ("home", fs.home),
                    ("documents", fs.documents),
                    ("downloads", fs.downloads),
                    ("pictures", fs.pictures),
                    ("music", fs.music),
                    ("videos", fs.videos),
                ] {
                    if on {
                        out.push(RevokedReach::FilesystemDir { dir: flag.into() });
                    }
                }
                // The label is set by a custom path too, so leaving these out
                // meant an app whose only filesystem grant was a custom path
                // showed "filesystem", offered a revoke, and gave up nothing.
                // Matched against the profile's own TOML entry, which is UTF-8
                // by definition, so a path that is not is one no revoke could
                // match. Skipped rather than lossily converted into a reach that
                // would silently hit nothing.
                for path in fs.custom.iter().filter_map(|p| p.to_str()) {
                    out.push(RevokedReach::FilesystemPath { path: path.to_string() });
                }
            }
            // Had no arm at all, so it fell through to the graph-prefix branch,
            // matched neither `read:` nor `write:`, and resolved to nothing -
            // the revoke reported success having removed no grant.
            "system" => {
                let sys = &profile.system;
                for (cap, on) in [
                    ("autostart", sys.autostart),
                    ("background", sys.background),
                    ("suspend", sys.power.suspend),
                    ("set_profile", sys.power.set_profile),
                ] {
                    if on {
                        out.push(RevokedReach::SystemCap { cap: cap.into() });
                    }
                }
            }
            "notifications" if profile.notifications.enabled => {
                out.push(RevokedReach::NotificationsOff);
            }
            "clipboard" => {
                for (cap, on) in [
                    ("read", profile.clipboard.read),
                    ("write", profile.clipboard.write),
                ] {
                    if on {
                        out.push(RevokedReach::ClipboardCap { cap: cap.into() });
                    }
                }
            }
            other => {
                // Graph scopes are carried verbatim in the label, so they need
                // no lookup - the label already names exactly one pattern.
                if let Some(pattern) = other.strip_prefix("read:") {
                    out.push(RevokedReach::Read {
                        entity_pattern: pattern.to_string(),
                    });
                } else if let Some(pattern) = other.strip_prefix("write:") {
                    out.push(RevokedReach::Write {
                        entity_pattern: pattern.to_string(),
                    });
                }
            }
        }
    }
    out
}

/// The labels this profile holds that revoking cannot currently take back.
///
/// Today that is a blanket `network.allow_all`: the revoke gate proves a strict
/// narrowing, and removing a domain from a list that `allow_all` overrides is
/// not one. Naming it is the point - a revoke button that quietly does nothing
/// for the broadest grant of all is worse than one that says it cannot.
pub fn unrevocable(profile: &arlen_permissions::PermissionProfile, labels: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if profile.network.allow_all && labels.iter().any(|l| l == "network") {
        out.push(
            "This app has blanket network access, which cannot be narrowed one domain at a time."
                .to_string(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Health;

    fn ext(id: &str, kind: ExtensionKind, caps: &[&str]) -> Extension {
        Extension {
            id: id.to_string(),
            name: id.to_string(),
            kind,
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            provenance: None,
            health: Health::Unknown,
        }
    }

    /// The invariant that was broken: every label `profile_labels` can put in
    /// front of a user must resolve to something a revoke can act on. A label
    /// that resolves to nothing is a button that reports success and gives up
    /// no grant, which is worse than having no button.
    ///
    /// `system` had no arm at all and `filesystem` ignored custom paths, so both
    /// produced exactly that.
    #[test]
    fn every_label_a_profile_can_show_resolves_to_a_reach() {
        let mut p = profile();
        p.network.allowed_domains = vec!["api.example.com".into()];
        p.filesystem.documents = true;
        p.filesystem.custom = vec![std::path::PathBuf::from("/opt/data")];
        p.notifications.enabled = true;
        p.clipboard.read = true;
        p.system.autostart = true;
        p.system.power.suspend = true;
        p.graph.read = vec!["system.File".to_string()];

        for label in crate::profile::profile_labels(&p) {
            let reaches = resolve_reaches(&p, std::slice::from_ref(&label));
            let excused = unrevocable(&p, std::slice::from_ref(&label));
            assert!(
                !reaches.is_empty() || !excused.is_empty(),
                "the label {label:?} resolves to no reach and is not reported unrevocable, \
                 so revoking it would silently do nothing"
            );
        }
    }

    /// The label is set by a custom path too, so the resolver has to produce one.
    #[test]
    fn a_custom_path_is_a_revocable_filesystem_reach() {
        let mut p = profile();
        p.filesystem.custom = vec![std::path::PathBuf::from("/opt/data")];
        assert_eq!(
            resolve_reaches(&p, &["filesystem".to_string()]),
            vec![RevokedReach::FilesystemPath { path: "/opt/data".to_string() }]
        );
    }

    /// Each granted system flag is its own reach, so revoking `system` gives up
    /// all of them rather than none.
    #[test]
    fn the_system_label_resolves_to_the_flags_that_are_set() {
        let mut p = profile();
        p.system.background = true;
        p.system.power.set_profile = true;
        assert_eq!(
            resolve_reaches(&p, &["system".to_string()]),
            vec![
                RevokedReach::SystemCap { cap: "background".to_string() },
                RevokedReach::SystemCap { cap: "set_profile".to_string() },
            ]
        );
    }

    /// Asking the user to confirm giving up nothing is friction with no content.
    #[test]
    fn an_extension_that_holds_nothing_has_nothing_to_revoke() {
        for kind in [ExtensionKind::App, ExtensionKind::Module, ExtensionKind::Bridge] {
            let p = plan(&ext("x", kind, &[]));
            assert!(p.is_empty(), "{kind:?}");
            assert!(p.residue.is_empty(), "{kind:?}");
        }
    }

    /// One word, three operations - the plan has to say which one ran.
    #[test]
    fn each_kind_plans_its_own_operation() {
        assert!(matches!(
            plan(&ext("org.example.App", ExtensionKind::App, &["network"])).steps[0],
            RevokeStep::NarrowProfile { .. }
        ));
        assert!(matches!(
            plan(&ext("com.example.Widget", ExtensionKind::Module, &["network"])).steps[0],
            RevokeStep::DropConsentGrants { .. }
        ));
        assert!(matches!(
            plan(&ext("md.obsidian", ExtensionKind::Bridge, &["write:md.obsidian"])).steps[0],
            RevokeStep::RemoveNamespaceGrant { .. }
        ));
    }

    /// No step reaches backwards, so a plan that claims nothing is left behind
    /// would be lying.
    #[test]
    fn every_revocable_extension_states_what_it_cannot_undo() {
        for kind in [ExtensionKind::App, ExtensionKind::Module, ExtensionKind::Bridge] {
            let p = plan(&ext("x", kind, &["write:a.b"]));
            assert!(!p.residue.is_empty(), "{kind:?} claimed a clean revoke");
        }
    }

    /// The residue a user is most likely to assume away: the grant goes, the
    /// data does not.
    #[test]
    fn a_bridge_says_its_ingested_data_stays() {
        let p = plan(&ext("md.obsidian", ExtensionKind::Bridge, &["write:md.obsidian"]));
        let text = p.residue.join(" ");
        assert!(text.contains("knowledge graph"), "{text}");
        assert!(text.contains("md.obsidian"), "{text}");
    }

    /// The namespace comes from the authority label, so the step cannot target
    /// something different from what the inventory showed.
    #[test]
    fn the_namespace_comes_from_the_granted_label() {
        let mut e = ext("display-name", ExtensionKind::Bridge, &["write:md.obsidian"]);
        e.name = "Obsidian".into();
        match &plan(&e).steps[0] {
            RevokeStep::RemoveNamespaceGrant { namespace } => {
                assert_eq!(namespace, "md.obsidian");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    fn profile() -> arlen_permissions::PermissionProfile {
        toml::from_str(
            "[info]\napp_id = \"org.example.App\"\nname = \"Example\"\ntier = \"third-party\"\n",
        )
        .expect("a minimal profile parses")
    }

    /// A label is the coarse answer, so giving it up means all of it - not the
    /// first domain, not the first directory.
    #[test]
    fn a_label_expands_to_every_grant_under_it() {
        let mut p = profile();
        p.network.allowed_domains = vec!["a.example.com".into(), "b.example.com".into()];
        p.filesystem.home = true;
        p.filesystem.documents = true;

        let reaches = resolve_reaches(&p, &["network".into(), "filesystem".into()]);
        assert_eq!(reaches.len(), 4, "{reaches:?}");
    }

    /// Nothing granted, nothing to revoke - not a step that would be refused.
    #[test]
    fn a_label_the_profile_does_not_hold_yields_no_step() {
        assert!(resolve_reaches(&profile(), &["filesystem".into(), "clipboard".into()]).is_empty());
    }

    /// The graph label already names the pattern, so it needs no lookup.
    #[test]
    fn a_graph_scope_carries_straight_through() {
        let got = resolve_reaches(&profile(), &["read:system.File".into()]);
        assert_eq!(
            got,
            vec![RevokedReach::Read {
                entity_pattern: "system.File".into()
            }]
        );
    }

    /// The revoke gate refuses removing a domain while `allow_all` makes the
    /// list moot, so producing those steps would generate refusals. It is
    /// reported as unrevocable instead - a button that quietly does nothing for
    /// the broadest grant of all is worse than one that says it cannot.
    #[test]
    fn blanket_network_access_is_reported_rather_than_uselessly_stepped() {
        let mut p = profile();
        p.network.allow_all = true;
        p.network.allowed_domains = vec!["a.example.com".into()];

        let labels = vec!["network".to_string()];
        assert!(resolve_reaches(&p, &labels).is_empty());
        assert!(!unrevocable(&p, &labels).is_empty());
    }

    /// With a plain allowlist there is nothing to warn about.
    #[test]
    fn an_ordinary_allowlist_is_fully_revocable() {
        let mut p = profile();
        p.network.allowed_domains = vec!["a.example.com".into()];
        let labels = vec!["network".to_string()];
        assert_eq!(resolve_reaches(&p, &labels).len(), 1);
        assert!(unrevocable(&p, &labels).is_empty());
    }
}
