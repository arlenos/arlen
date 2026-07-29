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

/// Decide whether an upgrade needs consent: a high-impact widening does; a
/// tightening, equal, or only-low-impact-widening update auto-applies.
///
/// Takes `Capabilities` on both sides because that is what the install lock
/// records. The old side needs no reconstruction from a manifest - by upgrade
/// time the manifest on disk is the NEW version, so deriving the baseline from
/// anything still installed would derive it from the wrong one.
pub fn upgrade_consent_from_caps(old: &Capabilities, new: &Capabilities) -> UpgradeConsent {
    let diff = diff_capabilities(old, new);
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

/// What an upgrade should do to the user (update-flow-plan.md, U-4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpgradeGate {
    /// Nothing new is being asked for. Apply without saying anything.
    ///
    /// The anti-fatigue half, and it earns its keep: a prompt on every update
    /// teaches people to click through prompts, so by the time one genuinely
    /// matters it is just another dialog in the way.
    Silent,
    /// The update wants something it did not have. Interrupt, name it, and let
    /// the user allow, skip this update, or uninstall.
    Interruptive {
        /// What is newly requested, in the diff's own plain-language wording.
        widened: Vec<String>,
    },
    /// Nothing was recorded for this app, so no comparison is possible.
    ///
    /// **Deliberately NOT Silent.** An unrecorded app might be widening
    /// enormously and we would have no way to tell; treating "we cannot check"
    /// as "nothing changed" is the exact fail-open the lock exists to prevent.
    /// The caller shows what the new version asks for and lets the user decide,
    /// as it would for a first install.
    Unknown {
        /// Everything the new version asks for, since none of it can be shown as
        /// a delta against a baseline that does not exist.
        requested: Vec<String>,
    },
}

/// Decide how an upgrade should meet the user.
///
/// `recorded` is what the install lock says was granted; `None` when the app has
/// no lock entry at all.
pub fn upgrade_gate(recorded: Option<&Capabilities>, new: &PermissionInfo) -> UpgradeGate {
    let new_caps = capabilities_from(new);
    let Some(old) = recorded else {
        // No baseline: describe the whole request rather than pretend it is a
        // no-op. Reuses the diff against nothing so the wording matches what an
        // ordinary widening would say.
        let against_nothing = diff_capabilities(&Capabilities::default(), &new_caps);
        return UpgradeGate::Unknown {
            requested: against_nothing
                .added
                .iter()
                .map(|c| c.description.clone())
                .collect(),
        };
    };

    match upgrade_consent_from_caps(old, &new_caps) {
        UpgradeConsent::AutoApply => UpgradeGate::Silent,
        UpgradeConsent::ConsentRequired(widened) => UpgradeGate::Interruptive { widened },
    }
}

/// What upgrading to the package at `path` would ask of the user, without
/// installing anything.
///
/// The store needs this BEFORE the user commits: "this update now wants X" is
/// only useful as a warning if it arrives while there is still a choice. It is
/// also why the answer is computed from the lock rather than from anything on
/// disk - after the upgrade the old manifest is gone.
///
/// The package is verified exactly as an install verifies it, in the same order,
/// before its manifest is read. A preview that reported the claims of an
/// unverified package would be reporting whatever the file felt like saying.
pub fn preview_upgrade(path: &str) -> Result<(String, UpgradeGate), crate::install::InstallError> {
    let temp_dir = crate::install::extract_package(path)?;
    crate::install::validate_package_structure(&temp_dir)?;
    crate::signature::verify_signature(&temp_dir)
        .map_err(|e| crate::install::InstallError::SignatureVerificationFailed(e.to_string()))?;

    let manifest = crate::install::load_manifest(&temp_dir)?;
    crate::install::validate_manifest(&manifest)?;
    let app_id = manifest.package.id.clone();

    // A lock we cannot read is not a licence to call the upgrade unchanged: an
    // unreadable baseline is exactly as uninformative as a missing one.
    let recorded = crate::lock::Lock::load(&crate::lock::lock_path())
        .ok()
        .and_then(|l| l.get(&app_id).map(|e| e.granted.clone()));

    let gate = upgrade_gate(recorded.as_ref(), &manifest.permissions);
    Ok((app_id, gate))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decision between two manifests, which is what the old wrapper did.
    fn consent_between(old: &PermissionInfo, new: &PermissionInfo) -> UpgradeConsent {
        upgrade_consent_from_caps(&capabilities_from(old), &capabilities_from(new))
    }

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
        assert_eq!(consent_between(&old, &equal), UpgradeConsent::AutoApply);
        // A tightening (drops a filesystem grant) is not a widening.
        let tighter = perms(&["~/Documents"], &["a.com"], &[]);
        assert_eq!(consent_between(&old, &tighter), UpgradeConsent::AutoApply);
    }

    /// The anti-fatigue half: an update that asks for nothing new must not
    /// interrupt. A prompt on every update teaches people to click through them.
    #[test]
    fn an_unchanged_or_narrowed_upgrade_is_silent() {
        let recorded = capabilities_from(&perms(&["~/Documents", "~/Downloads"], &["a.com"], &[]));

        let same = perms(&["~/Documents", "~/Downloads"], &["a.com"], &[]);
        assert_eq!(upgrade_gate(Some(&recorded), &same), UpgradeGate::Silent);

        let narrowed = perms(&["~/Documents"], &["a.com"], &[]);
        assert_eq!(upgrade_gate(Some(&recorded), &narrowed), UpgradeGate::Silent);
    }

    #[test]
    fn an_update_that_wants_more_interrupts_and_names_it() {
        let recorded = capabilities_from(&perms(&[], &[], &[]));
        let wider = perms(&["~/Documents"], &[], &[]);

        match upgrade_gate(Some(&recorded), &wider) {
            UpgradeGate::Interruptive { widened } => {
                assert!(!widened.is_empty(), "the user has to be told WHAT it wants");
            }
            other => panic!("a widening must interrupt, got {other:?}"),
        }
    }

    /// The fail-open this must not have: with nothing recorded we cannot tell
    /// whether the update widens, and calling that "unchanged" would wave through
    /// exactly the case the lock exists to catch.
    #[test]
    fn an_unrecorded_app_is_not_treated_as_unchanged() {
        let wants_a_lot = perms(&["~/Documents"], &["tracker.example"], &["system.File"]);

        match upgrade_gate(None, &wants_a_lot) {
            UpgradeGate::Unknown { requested } => {
                assert!(!requested.is_empty(), "the whole request has to be shown");
            }
            other => panic!("an unrecorded app must not be silent, got {other:?}"),
        }
    }

    /// The lock's stored grants feed the gate directly, with no reconstruction of
    /// the old manifest, which by upgrade time is the NEW one.
    #[test]
    fn the_gate_reads_the_lock_shape_directly() {
        let recorded = Capabilities {
            network: vec!["a.com".to_string()],
            ..Default::default()
        };
        let unchanged = perms(&[], &["a.com"], &[]);
        assert_eq!(upgrade_gate(Some(&recorded), &unchanged), UpgradeGate::Silent);
    }

    #[test]
    fn a_high_impact_widening_requires_consent() {
        // An update that newly requests a filesystem path (a high-impact add).
        let old = perms(&[], &[], &[]);
        let new = perms(&["~/Documents"], &[], &[]);
        match consent_between(&old, &new) {
            UpgradeConsent::ConsentRequired(deltas) => assert!(!deltas.is_empty()),
            UpgradeConsent::AutoApply => panic!("a new filesystem grant must require consent"),
        }
    }
}
