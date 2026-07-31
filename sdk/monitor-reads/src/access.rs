//! Per-app access aggregation (`system-monitor-plan.md` - the sovereign "who
//! touched what" lens). A read-only aggregation over the audit ledger's activity:
//! one row per acting app_id, summarising what each app did and how often, with
//! DENIED access as visible as granted. The AI agent is not special-cased - it is
//! one audited principal like any other, which is the point (the monitor audits
//! the AI exactly the way it audits every app).
//!
//! This is the audit-ledger half of per-app access. The real-time per-app RESOURCE
//! attribution (CPU/net by app) waits on the eBPF `pid -> app_id` bridge and is
//! surfaced as "not measured yet" until it lands.

use audit_proto::{ActivityPage, ReadClient};
use std::collections::{BTreeMap, BTreeSet};

/// One app's audited access, aggregated from the activity ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAccess {
    /// The `app_id` of the acting component (the audit `actor`).
    pub app_id: String,
    /// Total audited actions by this app in the page.
    pub total: usize,
    /// Count per action kind (`query` / `tool-call` / `graph-access` / ...).
    pub by_kind: BTreeMap<String, usize>,
    /// How many actions were DENIED - the anti-Recall signal: refused access is as
    /// visible as granted access.
    pub denied: usize,
    /// Earliest action timestamp seen in the page (micros since the Unix epoch).
    pub first_micros: i64,
    /// Latest action timestamp seen in the page (micros since the Unix epoch).
    pub last_micros: i64,
}

/// The per-app access report over a slice of the activity ledger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessReport {
    /// Whether the audit daemon answered. `false` means the data could not be read
    /// (fail-closed: the surface must show "cannot read", not "no access").
    pub available: bool,
    /// Whether the daemon reports its ledger tampered (the access data is suspect).
    pub tampered: bool,
    /// One row per acting app, sorted by `app_id`.
    pub apps: Vec<AppAccess>,
}

/// Aggregate an [`ActivityPage`] into per-app access summaries (pure), one row per
/// acting `app_id`, sorted by `app_id`. An empty page yields no rows.
pub fn aggregate_access(page: &ActivityPage) -> Vec<AppAccess> {
    let mut by_app: BTreeMap<String, AppAccess> = BTreeMap::new();
    for e in &page.entries {
        let row = by_app.entry(e.actor.clone()).or_insert_with(|| AppAccess {
            app_id: e.actor.clone(),
            total: 0,
            by_kind: BTreeMap::new(),
            denied: 0,
            first_micros: e.timestamp_micros,
            last_micros: e.timestamp_micros,
        });
        row.total += 1;
        *row.by_kind.entry(e.kind.clone()).or_insert(0) += 1;
        if e.outcome == "denied" {
            row.denied += 1;
        }
        row.first_micros = row.first_micros.min(e.timestamp_micros);
        row.last_micros = row.last_micros.max(e.timestamp_micros);
    }
    by_app.into_values().collect()
}

/// The first-time accesses in `current` against a `seen` baseline: every
/// `(app_id, kind)` pair present now but not in the baseline. This is the
/// plan's "an app reached the camera / network for the first time" alert. Pure;
/// the caller owns the persisted baseline and folds the returned pairs into it
/// after surfacing them. Works over the audit-activity kinds we have today
/// (graph-access, tool-call, network-call, ...); the capture kinds (camera, mic)
/// join it when the capture infrastructure lands. Pairs are returned sorted for a
/// stable surface.
pub fn novel_access(
    current: &[AppAccess],
    seen: &BTreeSet<(String, String)>,
) -> Vec<(String, String)> {
    let mut novel: Vec<(String, String)> = current
        .iter()
        .flat_map(|app| app.by_kind.keys().map(move |kind| (app.app_id.clone(), kind.clone())))
        .filter(|pair| !seen.contains(pair))
        .collect();
    novel.sort();
    novel
}

/// Read the recent activity and aggregate it into a per-app access report. The
/// page's `available` flag is carried through so the surface distinguishes "no app
/// accessed anything" from "the audit daemon could not be read".
pub async fn app_access(client: &ReadClient, limit: u64) -> AccessReport {
    let page = client.recent(limit).await;
    AccessReport {
        available: page.available,
        tampered: page.tampered,
        apps: aggregate_access(&page),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use audit_proto::ActivityEntry;

    fn entry(actor: &str, kind: &str, outcome: &str, ts: i64) -> ActivityEntry {
        ActivityEntry {
            index: 0,
            timestamp_micros: ts,
            kind: kind.to_string(),
            actor: actor.to_string(),
            subject: String::new(),
            outcome: outcome.to_string(),
            node_types: Vec::new(),
            relations: Vec::new(),
            result_count: None,
            duration_ms: None,
            depth: None,
            call_chain_id: None,
            project_id: None,
            entry_ref: String::new(),
        }
    }

    fn page(entries: Vec<ActivityEntry>) -> ActivityPage {
        ActivityPage { total: entries.len() as u64, entries, available: true, tampered: false }
    }

    #[test]
    fn groups_by_actor_with_kind_counts_and_time_span() {
        let p = page(vec![
            entry("files", "graph-access", "ok", 100),
            entry("files", "graph-access", "ok", 300),
            entry("files", "query", "ok", 200),
        ]);
        let rows = aggregate_access(&p);
        assert_eq!(rows.len(), 1);
        let files = &rows[0];
        assert_eq!(files.app_id, "files");
        assert_eq!(files.total, 3);
        assert_eq!(files.by_kind.get("graph-access"), Some(&2));
        assert_eq!(files.by_kind.get("query"), Some(&1));
        assert_eq!(files.first_micros, 100);
        assert_eq!(files.last_micros, 300);
    }

    #[test]
    fn denied_actions_are_counted_separately() {
        let p = page(vec![
            entry("app.x", "tool-call", "ok", 1),
            entry("app.x", "tool-call", "denied", 2),
            entry("app.x", "graph-access", "denied", 3),
        ]);
        let rows = aggregate_access(&p);
        assert_eq!(rows[0].total, 3);
        assert_eq!(rows[0].denied, 2);
    }

    #[test]
    fn the_ai_agent_is_one_row_like_any_other_principal() {
        let p = page(vec![
            entry("ai-agent", "graph-access", "ok", 10),
            entry("files", "graph-access", "ok", 20),
        ]);
        let rows = aggregate_access(&p);
        // Sorted by app_id, no special-casing: the AI is a plain audited row.
        assert_eq!(rows.iter().map(|r| r.app_id.as_str()).collect::<Vec<_>>(), vec!["ai-agent", "files"]);
    }

    #[test]
    fn an_empty_page_yields_no_rows() {
        assert!(aggregate_access(&page(Vec::new())).is_empty());
    }

    #[test]
    fn novel_access_flags_only_unseen_app_kind_pairs() {
        let rows = aggregate_access(&page(vec![
            entry("files", "graph-access", "ok", 1),
            entry("files", "query", "ok", 2),
            entry("mail", "network-call", "ok", 3),
        ]));
        let mut seen = BTreeSet::new();
        seen.insert(("files".to_string(), "graph-access".to_string()));
        let novel = novel_access(&rows, &seen);
        // files/graph-access is baseline; the other two are first-time, sorted.
        assert_eq!(
            novel,
            vec![
                ("files".to_string(), "query".to_string()),
                ("mail".to_string(), "network-call".to_string()),
            ]
        );
    }

    #[test]
    fn an_empty_baseline_makes_every_access_novel() {
        let rows = aggregate_access(&page(vec![entry("app.x", "tool-call", "ok", 1)]));
        assert_eq!(
            novel_access(&rows, &BTreeSet::new()),
            vec![("app.x".to_string(), "tool-call".to_string())]
        );
    }
}
