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
}
