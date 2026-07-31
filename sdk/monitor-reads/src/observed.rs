//! Observed-vs-declared capability use (`store-plan.md` §8.2, ST-7).
//!
//! An app declares what it may do. This answers the different question of what it
//! was actually seen doing on THIS machine, by reading the user's own audit
//! ledger. Nothing is uploaded, nothing is crowd-sourced, and no other machine's
//! behaviour is consulted.
//!
//! **The copy rule the plan is emphatic about: "not observed on your machine",
//! never "safe".** An app that has not yet exercised a grant still holds it, and
//! silence in a ledger is not evidence of restraint. So the vocabulary here is
//! deliberately about the observation, not about the app.
//!
//! The subtler half is that most declared capabilities have no audit feed at all.
//! Only graph access and network calls are audited today; a filesystem, clipboard
//! or input grant produces no entry when it is used. Reporting those as "not
//! observed" alongside the ones we genuinely watch would be the exact lie the copy
//! rule guards against, because the two are indistinguishable to a reader and one
//! of them is worthless. [`Observation::NotMeasured`] keeps them apart.

use serde::Serialize;
use std::collections::BTreeMap;

use crate::access::{AccessReport, AppAccess};

/// What the local ledger says about one declared capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Observation {
    /// Audited use of this capability appeared in the window that was read.
    Observed {
        /// How many audited actions evidenced it.
        actions: usize,
        /// The most recent one, micros since the Unix epoch.
        last_micros: i64,
    },
    /// This capability HAS an audit feed, and nothing in the window used it. The
    /// only state that carries the "not observed on your machine" copy - and it
    /// still says nothing about whether the app will use the grant tomorrow.
    NotObserved,
    /// Nothing on this machine records use of this capability, so silence about it
    /// is not evidence of anything. Distinct from [`Self::NotObserved`] because a
    /// surface that showed them the same way would be claiming a measurement it
    /// never took.
    NotMeasured {
        /// Why there is no measurement, for the surface to show verbatim rather
        /// than inventing its own wording.
        reason: NotMeasuredReason,
    },
}

/// Why a declared capability could not be measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotMeasuredReason {
    /// No audit kind evidences this capability. Using it is simply not recorded.
    NoFeed,
    /// There is a feed, but the ledger could not be read at all, so every
    /// capability is unmeasured this time rather than unused.
    LedgerUnavailable,
    /// The ledger records actions against an actor id, and which id THIS
    /// extension audits under is not established. A bridge is `bridge.<ns>` in
    /// the ledger but its bare namespace in the inventory, and a module's actor
    /// is not settled at all, so asking the ledger about the wrong id would come
    /// back empty and read as restraint.
    ActorUnknown,
    /// The ledger reports itself tampered. It may still answer, but an attacker
    /// who can edit it can remove exactly the entries that would have shown use,
    /// so absence of evidence is worth nothing here.
    LedgerTampered,
}

impl Observation {
    /// Whether this observation is positive evidence of use. Explicitly NOT the
    /// inverse of "safe": `false` covers both "we watched and saw nothing" and "we
    /// do not watch this at all".
    pub fn is_evidence_of_use(&self) -> bool {
        matches!(self, Self::Observed { .. })
    }
}

/// One declared capability with what the ledger says about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeclaredCapability {
    /// The capability label, in the shared vocabulary the store facets on
    /// (`network`, `filesystem`, `read:system.File`, ...).
    pub label: String,
    /// What was seen locally.
    pub observation: Observation,
}

/// The audit kinds that evidence a given capability label, or `None` when nothing
/// records its use.
///
/// Kept deliberately narrow. Adding a kind here claims "using this capability
/// produces that audit entry", and a wrong claim turns into a confident
/// "not observed" for a capability nobody is watching - worse than admitting the
/// gap. The strings are the activity-page labels (hyphenated), not
/// `AuditKind::as_str` (underscored); they differ, and matching the wrong set
/// would silently evidence nothing.
fn feed_for(label: &str) -> Option<&'static [&'static str]> {
    // A graph scope, `read:system.File` / `write:system.Project`. Graph access is
    // audited, so these are genuinely measured.
    if label.starts_with("read:") || label.starts_with("write:") {
        return Some(&["graph-access"]);
    }
    match label {
        "network" => Some(&["network-call"]),
        // Everything else - filesystem, clipboard, notifications, input, search,
        // intents, events, system - has no audit entry emitted at the point of
        // use. They are declared and enforced, just not recorded.
        _ => None,
    }
}

/// Compare an app's declared capabilities against what the local ledger observed.
///
/// `declared` is the app's capability footprint (`arlen_extensions::profile_labels`
/// and its siblings). `report` is a page of the user's own audit ledger. Returns
/// one row per declared capability, in the order declared, so a caller that sorted
/// its labels keeps that order.
///
/// A declared capability the app never exercised still appears - the surface is
/// about the grant, and hiding unexercised grants would make an over-broad app
/// look narrow.
pub fn observed_vs_declared(
    declared: &[String],
    report: &AccessReport,
    app_id: &str,
) -> Vec<DeclaredCapability> {
    // Fail honest, not fail quiet. If the ledger did not answer, or answered while
    // reporting itself tampered, then nothing was measured this time - including
    // the capabilities that do have feeds. Reporting those as "not observed" would
    // present a failed read as a clean bill.
    let ledger_fault = if !report.available {
        Some(NotMeasuredReason::LedgerUnavailable)
    } else if report.tampered {
        Some(NotMeasuredReason::LedgerTampered)
    } else {
        None
    };

    let app = report.apps.iter().find(|a| a.app_id == app_id);

    declared
        .iter()
        .map(|label| DeclaredCapability {
            label: label.clone(),
            observation: match (feed_for(label), ledger_fault) {
                (None, _) => Observation::NotMeasured {
                    reason: NotMeasuredReason::NoFeed,
                },
                (Some(_), Some(reason)) => Observation::NotMeasured { reason },
                (Some(kinds), None) => observe(kinds, app),
            },
        })
        .collect()
}

/// Total the app's actions across the kinds that evidence one capability.
fn observe(kinds: &[&str], app: Option<&AppAccess>) -> Observation {
    // No row for this app means it did nothing audited at all in the window, which
    // for a fed capability is a real "not observed" rather than a missing reading.
    let Some(app) = app else {
        return Observation::NotObserved;
    };
    let actions: usize = kinds.iter().filter_map(|k| app.by_kind.get(*k)).sum();
    if actions == 0 {
        return Observation::NotObserved;
    }
    // `AppAccess` times the app's whole activity, not per kind, so the last
    // timestamp is the app's last audited action of any sort. Reporting it as the
    // last use of THIS capability would be a small lie on a surface whose whole
    // point is not overclaiming, so it is only carried when every audited action
    // in the window belongs to the kinds being asked about.
    let last_micros = if app.total == actions { app.last_micros } else { 0 };
    Observation::Observed {
        actions,
        last_micros,
    }
}

/// Every declared capability marked unmeasured for one reason, for a caller that
/// cannot name the actor this extension audits under.
///
/// The alternative is passing a guessed id into [`observed_vs_declared`], which
/// answers "not observed" for every fed capability because the ledger has no such
/// actor - a confident wrong reading rather than an admitted gap.
pub fn all_unmeasured(declared: &[String], reason: NotMeasuredReason) -> Vec<DeclaredCapability> {
    declared
        .iter()
        .map(|label| DeclaredCapability {
            label: label.clone(),
            observation: Observation::NotMeasured { reason },
        })
        .collect()
}

/// Group rows by observation for a surface that wants counts rather than a list -
/// "2 observed, 1 not observed, 5 not measured".
pub fn tally(rows: &[DeclaredCapability]) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for row in rows {
        let key = match row.observation {
            Observation::Observed { .. } => "observed",
            Observation::NotObserved => "not-observed",
            Observation::NotMeasured { .. } => "not-measured",
        };
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(app_id: &str, kinds: &[(&str, usize)], total: usize) -> AppAccess {
        AppAccess {
            app_id: app_id.to_string(),
            total,
            by_kind: kinds.iter().map(|(k, n)| (k.to_string(), *n)).collect(),
            denied: 0,
            first_micros: 10,
            last_micros: 500,
        }
    }

    fn report(apps: Vec<AppAccess>) -> AccessReport {
        AccessReport {
            available: true,
            tampered: false,
            apps,
        }
    }

    fn labels(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_capability_with_no_feed_is_never_reported_as_unused() {
        // The heart of the copy rule. Nothing audits a filesystem or clipboard
        // read, so an app doing it constantly looks identical to one that never
        // does. Saying "not observed" there would invite exactly the "so it is
        // safe" reading the plan forbids.
        let rows = observed_vs_declared(
            &labels(&["filesystem", "clipboard", "input", "notifications"]),
            &report(vec![]),
            "org.example.App",
        );
        assert_eq!(rows.len(), 4);
        for row in &rows {
            assert_eq!(
                row.observation,
                Observation::NotMeasured {
                    reason: NotMeasuredReason::NoFeed
                },
                "{} has no audit feed",
                row.label
            );
            assert!(!row.observation.is_evidence_of_use());
        }
    }

    #[test]
    fn a_fed_capability_the_app_never_used_is_not_observed() {
        let rows = observed_vs_declared(
            &labels(&["network"]),
            &report(vec![app("org.example.App", &[("graph-access", 3)], 3)]),
            "org.example.App",
        );
        assert_eq!(rows[0].observation, Observation::NotObserved);
    }

    #[test]
    fn an_app_absent_from_the_ledger_is_not_observed_rather_than_unmeasured() {
        // It did nothing audited at all, which for a fed capability is a genuine
        // reading and not a missing one.
        let rows = observed_vs_declared(
            &labels(&["network", "read:system.File"]),
            &report(vec![app("other.app", &[("network-call", 9)], 9)]),
            "org.example.App",
        );
        assert!(rows.iter().all(|r| r.observation == Observation::NotObserved));
    }

    #[test]
    fn use_of_a_fed_capability_is_counted() {
        let rows = observed_vs_declared(
            &labels(&["network"]),
            &report(vec![app("org.example.App", &[("network-call", 4)], 4)]),
            "org.example.App",
        );
        assert_eq!(
            rows[0].observation,
            Observation::Observed {
                actions: 4,
                last_micros: 500
            }
        );
        assert!(rows[0].observation.is_evidence_of_use());
    }

    #[test]
    fn a_last_use_time_is_withheld_when_it_would_belong_to_another_capability() {
        // The aggregate times the app, not the kind. With mixed activity the last
        // action may well have been the graph read, so attributing it to the
        // network grant would overstate what is known.
        let rows = observed_vs_declared(
            &labels(&["network"]),
            &report(vec![app(
                "org.example.App",
                &[("network-call", 2), ("graph-access", 7)],
                9,
            )]),
            "org.example.App",
        );
        assert_eq!(
            rows[0].observation,
            Observation::Observed {
                actions: 2,
                last_micros: 0
            }
        );
    }

    #[test]
    fn graph_scopes_are_measured_by_graph_access() {
        let rows = observed_vs_declared(
            &labels(&["read:system.File", "write:system.Project"]),
            &report(vec![app("org.example.App", &[("graph-access", 6)], 6)]),
            "org.example.App",
        );
        for row in &rows {
            assert_eq!(
                row.observation,
                Observation::Observed {
                    actions: 6,
                    last_micros: 500
                },
                "{} is evidenced by graph access",
                row.label
            );
        }
    }

    #[test]
    fn an_unreadable_ledger_measures_nothing_rather_than_clearing_the_app() {
        // The failure this exists to prevent: a daemon that is down producing a
        // page with no entries, which naively reads as "this app used nothing".
        let unavailable = AccessReport {
            available: false,
            tampered: false,
            apps: vec![],
        };
        let rows = observed_vs_declared(&labels(&["network"]), &unavailable, "org.example.App");
        assert_eq!(
            rows[0].observation,
            Observation::NotMeasured {
                reason: NotMeasuredReason::LedgerUnavailable
            }
        );
    }

    #[test]
    fn a_tampered_ledger_measures_nothing_even_though_it_answered() {
        // Someone able to edit the ledger would remove the entries showing use, so
        // an answer from a tampered ledger is worth less than no answer.
        let tampered = AccessReport {
            available: true,
            tampered: true,
            apps: vec![app("org.example.App", &[("network-call", 1)], 1)],
        };
        let rows = observed_vs_declared(&labels(&["network"]), &tampered, "org.example.App");
        assert_eq!(
            rows[0].observation,
            Observation::NotMeasured {
                reason: NotMeasuredReason::LedgerTampered
            }
        );
    }

    #[test]
    fn every_declared_capability_gets_a_row_in_the_order_declared() {
        // Dropping unexercised grants would make an over-broad app look narrow,
        // which inverts the surface's purpose.
        let declared = labels(&["network", "filesystem", "read:system.File"]);
        let rows = observed_vs_declared(&declared, &report(vec![]), "org.example.App");
        assert_eq!(
            rows.iter().map(|r| r.label.as_str()).collect::<Vec<_>>(),
            vec!["network", "filesystem", "read:system.File"]
        );
    }

    #[test]
    fn the_tally_separates_the_three_states() {
        let rows = observed_vs_declared(
            &labels(&["network", "read:system.File", "filesystem", "clipboard"]),
            &report(vec![app("org.example.App", &[("network-call", 2)], 2)]),
            "org.example.App",
        );
        let counts = tally(&rows);
        assert_eq!(counts.get("observed"), Some(&1));
        assert_eq!(counts.get("not-observed"), Some(&1));
        assert_eq!(counts.get("not-measured"), Some(&2));
    }

    #[test]
    fn nothing_declared_yields_nothing() {
        assert!(observed_vs_declared(&[], &report(vec![]), "org.example.App").is_empty());
    }
}
