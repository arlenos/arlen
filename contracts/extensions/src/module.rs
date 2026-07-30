//! A module's capability footprint, in the shared vocabulary.
//!
//! Distinct from `modulesd`'s consent classifier, which turns the same
//! manifest into a sentence for a dialog ("can read your clipboard") and a
//! severity. This produces facet LABELS. They must not be merged: the dialog
//! wants prose the user reads once at the moment of decision, and the
//! inventory wants a token that filters identically across apps, modules and
//! bridges.

use std::collections::BTreeSet;

use arlen_modules::ModuleCapabilities;

/// The coarse capability labels a module manifest implies.
///
/// The thresholds match [`crate::profile::profile_labels`] deliberately: a
/// module reaching the network and an app reaching the network answer the same
/// filter, so both cross at "any reach at all" rather than one counting hosts
/// and the other counting whether.
///
/// Absence of a declaration is absence of a label. A module that declares
/// nothing yields nothing, which is what makes "asks for least" a real ordering
/// rather than an artefact of which manifest was most verbose.
pub fn module_labels(caps: &ModuleCapabilities) -> Vec<String> {
    let mut labels: BTreeSet<String> = BTreeSet::new();

    if caps.network.as_ref().is_some_and(|n| !n.allowed_domains.is_empty()) {
        labels.insert("network".into());
    }
    // A module's storage quota IS its filesystem reach: it has no path grants,
    // only its own quota-bounded area, so any quota at all is the same coarse
    // answer an app's directory grant gives.
    if caps.storage.is_some() {
        labels.insert("filesystem".into());
    }
    if caps.notifications {
        labels.insert("notifications".into());
    }
    // Either direction, matching the profile labeller: reading is the privacy
    // question, but writing can displace what the user copied.
    if caps.clipboard.as_ref().is_some_and(|c| c.read || c.write) {
        labels.insert("clipboard".into());
    }
    // Event-bus reach is how a module observes the rest of the session, which
    // is the same "runs alongside your system" answer `system` carries for an
    // app's autostart and background grants.
    if caps
        .event_bus
        .as_ref()
        .is_some_and(|b| !b.subscribe.is_empty() || !b.publish.is_empty())
    {
        labels.insert("system".into());
    }
    if let Some(graph) = &caps.graph {
        for scope in &graph.read {
            labels.insert(format!("read:{scope}"));
        }
        for scope in &graph.write {
            labels.insert(format!("write:{scope}"));
        }
    }

    labels.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arlen_modules::{
        ClipboardCapability, EventBusCapability, GraphCapability, NetworkCapability,
        StorageCapability,
    };

    #[test]
    fn a_module_that_declares_nothing_has_no_labels() {
        assert!(module_labels(&ModuleCapabilities::default()).is_empty());
    }

    /// The whole point of the shared crate: an app and a module that both reach
    /// the network must answer the same filter.
    #[test]
    fn a_module_and_an_app_reaching_the_network_carry_the_same_label() {
        let m = ModuleCapabilities {
            network: Some(NetworkCapability {
                allowed_domains: vec!["api.example.com".into()],
            }),
            ..Default::default()
        };

        let mut p: arlen_permissions::PermissionProfile = toml::from_str(
            "[info]\napp_id = \"org.example.App\"\nname = \"Example\"\ntier = \"third-party\"\n",
        )
        .expect("a minimal profile parses");
        p.network.allowed_domains = vec!["api.example.com".into()];

        assert_eq!(module_labels(&m), crate::profile::profile_labels(&p));
    }

    /// Each flag in these OR chains must earn its label alone, the same property
    /// the profile labeller has. Mutation testing found the `||` in the clipboard
    /// and event-bus checks could be flipped to `&&` unnoticed, which would mean
    /// a module with read-only clipboard access showed nothing at all.
    #[test]
    fn one_grant_alone_earns_the_module_label() {
        let clip = |read, write| ModuleCapabilities {
            clipboard: Some(ClipboardCapability { read, write }),
            ..Default::default()
        };
        let bus = |subscribe: Vec<String>, publish: Vec<String>| ModuleCapabilities {
            event_bus: Some(EventBusCapability { subscribe, publish }),
            ..Default::default()
        };
        let cases: Vec<(&str, ModuleCapabilities)> = vec![
            ("clipboard", clip(true, false)),
            ("clipboard", clip(false, true)),
            ("system", bus(vec!["a.*".into()], Vec::new())),
            ("system", bus(Vec::new(), vec!["a.*".into()])),
        ];
        for (i, (label, caps)) in cases.iter().enumerate() {
            let labels = module_labels(caps);
            assert!(
                labels.iter().any(|l| l == label),
                "case {i}: this single grant alone must show {label}, got {labels:?}"
            );
        }
    }

    /// Graph scopes stay verbatim so a `read:system.File` facet matches a
    /// module exactly as it matches an app or a bridge.
    #[test]
    fn graph_scopes_are_carried_through_unchanged() {
        let m = ModuleCapabilities {
            graph: Some(GraphCapability {
                read: vec!["system.File".into()],
                write: vec!["md.obsidian.Note".into()],
            }),
            ..Default::default()
        };
        let labels = module_labels(&m);
        assert!(labels.contains(&"read:system.File".to_string()), "{labels:?}");
        assert!(
            labels.contains(&"write:md.obsidian.Note".to_string()),
            "{labels:?}"
        );
    }

    #[test]
    fn the_labels_are_sorted_and_deduped() {
        let m = ModuleCapabilities {
            notifications: true,
            storage: Some(StorageCapability { quota_mb: 10 }),
            clipboard: Some(ClipboardCapability {
                read: true,
                write: true,
            }),
            event_bus: Some(EventBusCapability {
                subscribe: vec!["window.".into()],
                publish: vec!["window.".into()],
            }),
            ..Default::default()
        };
        let labels = module_labels(&m);
        let mut sorted = labels.clone();
        sorted.sort();
        assert_eq!(labels, sorted);
        // clipboard, filesystem, notifications, system - each once.
        assert_eq!(labels.len(), 4, "{labels:?}");
    }
}
