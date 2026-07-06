//! Capability-diff consent gate for `.lunpkg` upgrades (app-enrollment §E6).
//!
//! On an upgrade, installd diffs the new package's declared capabilities against
//! the installed ones. A high-impact WIDENING surfaces a consent moment (the S16
//! high-impact-always-confirm doctrine); a conservative-or-tightening update
//! applies silently (the anti-permission-fatigue discipline). Low-impact
//! additions are granted at the ceiling and prompt on first use instead.
//!
//! This is the BACKEND half: the type bridge from the `.lunpkg` `PermissionInfo`
//! to the recipe `Capabilities` that `diff_capabilities` compares, the diff on
//! the upgrade path, and the consent-required signal. The UI that renders the
//! prompt is the unified consent dialog (deferred).

use std::collections::BTreeMap;

use arlen_forage_capabilities::diff_capabilities;
use arlen_forage_recipe::Capabilities;

use crate::install::PermissionInfo;

/// Bridge a `.lunpkg` manifest's flat [`PermissionInfo`] to the recipe
/// [`Capabilities`] shape `diff_capabilities` compares. The manifest's separate
/// graph read/write lists become `read:`/`write:` scope strings (the recipe graph
/// grammar); `input` requests are preserved under `extra` so a newly-requested
/// global-input capability still shows up in the diff; the manifest carries no
/// `audio`, so it defaults false.
pub fn capabilities_from(perms: &PermissionInfo) -> Capabilities {
    let mut graph = Vec::with_capacity(perms.graph_read.len() + perms.graph_write.len());
    for r in &perms.graph_read {
        graph.push(format!("read:{r}"));
    }
    for w in &perms.graph_write {
        graph.push(format!("write:{w}"));
    }
    let mut extra = BTreeMap::new();
    for i in &perms.input {
        extra.insert(format!("input:{i}"), toml::Value::Boolean(true));
    }
    Capabilities {
        filesystem: perms.filesystem.clone(),
        network: perms.network.clone(),
        graph,
        notifications: perms.notifications,
        clipboard: perms.clipboard,
        audio: false,
        extra,
    }
}

/// The consent decision for an upgrade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeConsent {
    /// Conservative or tightening (or only low-impact additions): apply silently.
    AutoApply,
    /// A high-impact widening: the listed capability additions require explicit
    /// consent before the upgrade may proceed (the "grew since last version"
    /// delta the unified consent dialog renders).
    ConsentRequired(Vec<String>),
}

/// Decide whether upgrading from `old` to `new` (both the `.lunpkg`
/// `PermissionInfo`) needs consent. A high-impact widening requires it; a
/// tightening, equal, or only-low-impact-widening update auto-applies.
pub fn upgrade_consent(old: &PermissionInfo, new: &PermissionInfo) -> UpgradeConsent {
    let diff = diff_capabilities(&capabilities_from(old), &capabilities_from(new));
    if diff.requires_consent() {
        let deltas = diff
            .added
            .iter()
            .filter(|c| c.high_impact)
            .map(|c| c.description.clone())
            .collect();
        UpgradeConsent::ConsentRequired(deltas)
    } else {
        UpgradeConsent::AutoApply
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(filesystem: &[&str], network: &[&str], graph_read: &[&str]) -> PermissionInfo {
        PermissionInfo {
            graph_read: graph_read.iter().map(|s| s.to_string()).collect(),
            graph_write: vec![],
            filesystem: filesystem.iter().map(|s| s.to_string()).collect(),
            network: network.iter().map(|s| s.to_string()).collect(),
            notifications: false,
            clipboard: false,
            input: vec![],
        }
    }

    #[test]
    fn the_bridge_reconstructs_recipe_graph_scopes() {
        let p = perms(&["~/Documents"], &["api.example.com"], &["system.File"]);
        let caps = capabilities_from(&p);
        assert_eq!(caps.filesystem, vec!["~/Documents"]);
        assert_eq!(caps.network, vec!["api.example.com"]);
        assert_eq!(caps.graph, vec!["read:system.File"]);
    }

    #[test]
    fn a_tightening_or_equal_upgrade_auto_applies() {
        let old = perms(&["~/Documents", "~/Downloads"], &["a.com"], &[]);
        let equal = old.clone();
        assert_eq!(upgrade_consent(&old, &equal), UpgradeConsent::AutoApply);
        // A tightening (drops a filesystem grant) is not a widening.
        let tighter = perms(&["~/Documents"], &["a.com"], &[]);
        assert_eq!(upgrade_consent(&old, &tighter), UpgradeConsent::AutoApply);
    }

    #[test]
    fn a_high_impact_widening_requires_consent() {
        // An update that newly requests a filesystem path (a high-impact add).
        let old = perms(&[], &[], &[]);
        let new = perms(&["~/Documents"], &[], &[]);
        match upgrade_consent(&old, &new) {
            UpgradeConsent::ConsentRequired(deltas) => assert!(!deltas.is_empty()),
            UpgradeConsent::AutoApply => panic!("a new filesystem grant must require consent"),
        }
    }
}
