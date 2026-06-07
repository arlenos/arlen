//! Filling a [`SystemSnapshot`] from the Knowledge Graph.
//!
//! Foundation §5.8 builds the explanation from two sources: the **live
//! event stream** (the current moment: active processes, open network
//! connections) and the **Knowledge Graph** (context: which project the
//! files belong to, what is normal). This module fills the **graph
//! context half** behind a read-only [`GraphReader`] seam.
//!
//! What the graph cleanly provides today is bounded by what the
//! knowledge daemon promotes into queryable nodes: only `file.opened`
//! and `window.focused` are promoted, so the graph yields **recent file
//! activity** (with the accessing app and owning project) and the
//! **active project**. It does *not* yield current processes or open
//! sockets (those events are stored but not promoted to nodes) or
//! anomaly findings (the Anomaly Detector owns those). Those fields of
//! the snapshot are filled by their own sources and folded in by the
//! caller; this module never fabricates them.
//!
//! Every value read here is treated as data, not instruction: the
//! prompt builder wraps the whole snapshot in a tagged `GRAPH-DATA`
//! block, so a hostile file path cannot influence the model.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::snapshot::{
    Anomaly, AnomalyKind, Coverage, FileActivity, ProjectContext, SystemSnapshot,
};

/// How many recent file accesses to include in the snapshot. A small
/// cap keeps the prompt bounded; the explanation summarises rather than
/// enumerates.
pub const RECENT_FILES_LIMIT: usize = 12;

/// The graph node and edge labels [`graph_context`] reads. A caller
/// that enforces a capability scope (the ai-daemon) must check that its
/// scope permits **every** one of these before invoking the
/// explanation, and fail closed otherwise: the fixed queries below
/// touch all of them, so a narrower read tier that omits any one would
/// otherwise leak labels it does not grant. Kept here, next to the
/// queries, so the list cannot drift from what they actually read (a
/// drift-guard test asserts each label appears in a query).
pub const REQUIRED_GRAPH_LABELS: &[&str] =
    &["File", "App", "Project", "ACCESSED_BY", "FILE_PART_OF"];

/// The recency horizon for "current" file activity, in seconds. The
/// question is "what is my computer doing right *now*", so the snapshot
/// only includes files accessed within this window; older graph entries
/// are historical context, not current activity, and must not be
/// rendered as if they were happening now.
pub const RECENT_WINDOW_SECS: i64 = 6 * 3600;

/// Graph timestamps (`File.last_accessed`) are stored in **microseconds**
/// since the Unix epoch (producers stamp events with `as_micros()`), so
/// a `now_unix` in seconds is scaled to micros for the cutoff comparison.
const MICROS_PER_SEC: i64 = 1_000_000;

/// An error reading a source for a snapshot.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// The graph read failed at the transport or query level.
    #[error("graph read failed: {0}")]
    GraphRead(String),
    /// The anomaly source failed at the I/O level (a malformed or missing
    /// findings file is not an error; it reads as no anomalies).
    #[error("anomaly read failed: {0}")]
    AnomalyRead(String),
}

/// Read-only, typed Knowledge Graph access: one map per row keyed by
/// column name, each cell a JSON value (the daemon's structured-row
/// mode). The seam keeps [`graph_context`] testable with a mock and
/// decoupled from the concrete socket client.
#[async_trait]
pub trait GraphReader: Send + Sync {
    /// Run a read-only Cypher query and return typed rows.
    async fn query_rows(&self, cypher: &str) -> Result<Vec<HashMap<String, Value>>, SnapshotError>;
}

/// The Cypher for recent file activity: files accessed within the
/// recency window, most recent first, with their app and (optional)
/// owning project. Both the cutoff (a derived integer) and the limit (a
/// fixed constant) are inlined integers, so the query is injection-safe.
fn recent_files_query(cutoff_micros: i64) -> String {
    format!(
        "MATCH (f:File)-[:ACCESSED_BY]->(a:App) \
         WHERE f.last_accessed >= {cutoff_micros} \
         OPTIONAL MATCH (f)-[:FILE_PART_OF]->(p:Project) \
         RETURN f.path AS path, a.name AS app, p.name AS project, \
                f.last_accessed AS last_accessed \
         ORDER BY f.last_accessed DESC LIMIT {RECENT_FILES_LIMIT}"
    )
}

/// The Cypher for the active project: the most recently accessed project
/// that is both promoted and `active` (an archived project stays
/// promoted, so the status filter is required to avoid reporting it as
/// current), and how many distinct files the graph associates with it.
/// "Active" here is the KG proxy (most recently accessed), distinct from
/// the shell's Focus-Mode project.
const ACTIVE_PROJECT_QUERY: &str =
    "MATCH (p:Project) WHERE p.promoted = true AND p.status = 'active' \
     OPTIONAL MATCH (f:File)-[:FILE_PART_OF]->(p) \
     RETURN p.name AS name, p.last_accessed AS last_accessed, \
            count(DISTINCT f) AS file_count \
     ORDER BY last_accessed DESC LIMIT 1";

/// Read a string cell, treating a missing or non-string value (e.g. a
/// Cypher NULL from an `OPTIONAL MATCH`) as absent.
fn str_cell(row: &HashMap<String, Value>, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// Read a non-negative integer cell, accepting both unsigned and signed
/// JSON integers; a missing or negative value reads as 0.
fn u64_cell(row: &HashMap<String, Value>, key: &str) -> u64 {
    match row.get(key) {
        Some(v) if v.is_u64() => v.as_u64().unwrap_or(0),
        Some(v) if v.is_i64() => u64::try_from(v.as_i64().unwrap_or(0)).unwrap_or(0),
        _ => 0,
    }
}

/// Turn recent-file rows into [`FileActivity`]. A row without a `path`
/// or `app` is skipped (a malformed row never becomes a half-empty
/// entry).
fn parse_files(rows: &[HashMap<String, Value>]) -> Vec<FileActivity> {
    rows.iter()
        .filter_map(|row| {
            let path = str_cell(row, "path")?;
            let app = str_cell(row, "app")?;
            Some(FileActivity {
                path,
                app,
                project: str_cell(row, "project"),
            })
        })
        .collect()
}

/// Turn the active-project row (at most one) into a [`ProjectContext`].
fn parse_active_project(rows: &[HashMap<String, Value>]) -> Option<ProjectContext> {
    let row = rows.first()?;
    let name = str_cell(row, "name")?;
    Some(ProjectContext {
        name,
        file_count: u64_cell(row, "file_count"),
    })
}

/// Build the **graph-context half** of a snapshot: `files` (accessed
/// within [`RECENT_WINDOW_SECS`] of `now_unix`) and `active_project`,
/// read through `reader`. `now_unix` (Unix **seconds**) stamps the
/// snapshot and derives the recency cutoff, supplied by the caller so
/// this stays clock-free and testable. The returned snapshot's
/// [`Coverage::graph_context`] is set; `processes`, `network`, and
/// `anomalies` are left empty with their coverage flags false, for their
/// own sources to fill.
pub async fn graph_context(
    reader: &dyn GraphReader,
    now_unix: i64,
) -> Result<SystemSnapshot, SnapshotError> {
    // Files older than the window are historical context, not current
    // activity. A non-positive cutoff (clock at/before the epoch) floors
    // at 0, which matches everything rather than excluding everything.
    let cutoff_micros = now_unix
        .saturating_mul(MICROS_PER_SEC)
        .saturating_sub(RECENT_WINDOW_SECS.saturating_mul(MICROS_PER_SEC))
        .max(0);
    let file_rows = reader.query_rows(&recent_files_query(cutoff_micros)).await?;
    let project_rows = reader.query_rows(ACTIVE_PROJECT_QUERY).await?;
    Ok(SystemSnapshot {
        captured_at_unix: now_unix,
        files: parse_files(&file_rows),
        active_project: parse_active_project(&project_rows),
        coverage: Coverage {
            graph_context: true,
            live_processes: false,
            anomalies: false,
        },
        ..Default::default()
    })
}

/// Production [`GraphReader`] over the knowledge daemon's read socket.
///
/// Each query uses a fresh `os_sdk::UnixGraphClient` (fresh connection),
/// keeping the reader stateless; the explanation query rate is low (it
/// runs only when the user asks), so the reconnect cost is irrelevant.
pub struct UnixGraphReader {
    socket_path: String,
}

impl UnixGraphReader {
    /// Build a reader for the given knowledge query socket.
    pub fn new(socket_path: impl Into<String>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }
}

#[async_trait]
impl GraphReader for UnixGraphReader {
    async fn query_rows(&self, cypher: &str) -> Result<Vec<HashMap<String, Value>>, SnapshotError> {
        os_sdk::UnixGraphClient::new(self.socket_path.clone())
            .query_rows(cypher)
            .await
            .map_err(|e| SnapshotError::GraphRead(e.to_string()))
    }
}

/// Reads flagged anomalies for the explanation (Foundation §5.8, the
/// "is this normal" half the Anomaly Detector owns). Behind a seam so the
/// snapshot stays testable with a mock and decoupled from the detector's
/// on-disk format.
pub trait AnomalyReader: Send + Sync {
    /// Return the currently-flagged anomalies, or empty if none. A missing
    /// or unreadable findings store is not an error, it is no anomalies.
    fn read_anomalies(&self) -> Result<Vec<Anomaly>, SnapshotError>;
}

/// Map an Anomaly Detector finding (its stable `kind` string plus summary)
/// onto the explanation's typed anomaly. The detector's kinds are broader
/// than Foundation's two named ones, so anything that is not clearly a
/// novel-node or undeclared-network finding maps to `UnusualForContext`,
/// the catch-all "unusual for this machine" bucket, rather than being
/// dropped. Pure, so the mapping is unit-tested.
fn anomaly_from_alert(kind: &str, summary: &str) -> Anomaly {
    let k = kind.to_ascii_lowercase();
    let mapped = if k.contains("novel") || k.contains("node") {
        AnomalyKind::NovelNodeAccess
    } else if k.contains("network") || k.contains("destination") {
        AnomalyKind::UndeclaredNetworkDestination
    } else {
        AnomalyKind::UnusualForContext
    };
    Anomaly {
        kind: mapped,
        description: summary.to_string(),
    }
}

/// Build the **anomaly half** of a snapshot through `reader`. Sets
/// [`Coverage::anomalies`]; the graph and live fields are left empty for
/// their own sources, to be combined with [`merge_snapshots`].
pub fn anomaly_context(
    reader: &dyn AnomalyReader,
    now_unix: i64,
) -> Result<SystemSnapshot, SnapshotError> {
    Ok(SystemSnapshot {
        captured_at_unix: now_unix,
        anomalies: reader.read_anomalies()?,
        coverage: Coverage {
            graph_context: false,
            live_processes: false,
            anomalies: true,
        },
        ..Default::default()
    })
}

/// Production [`AnomalyReader`] over the Anomaly Detector's persisted
/// findings file (`alerts.json`, a `{ "alerts": [ { kind, summary, .. } ] }`
/// document). The path is supplied by the caller rather than resolved here,
/// so the crate stays free of environment assumptions. The file is parsed
/// liberally: a missing file, a parse error, or an entry without a `kind`
/// yields no (or one fewer) anomaly rather than failing the explanation,
/// since the source is advisory.
pub struct FileAnomalyReader {
    path: String,
}

impl FileAnomalyReader {
    /// Build a reader for the given findings-file path.
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }
}

impl AnomalyReader for FileAnomalyReader {
    fn read_anomalies(&self) -> Result<Vec<Anomaly>, SnapshotError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(SnapshotError::AnomalyRead(e.to_string())),
        };
        // A malformed file is advisory-degraded to no anomalies, not an error.
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return Ok(Vec::new());
        };
        let Some(alerts) = value.get("alerts").and_then(Value::as_array) else {
            return Ok(Vec::new());
        };
        Ok(alerts
            .iter()
            .filter_map(|a| {
                let kind = a.get("kind").and_then(Value::as_str)?;
                let summary = a.get("summary").and_then(Value::as_str).unwrap_or("");
                Some(anomaly_from_alert(kind, summary))
            })
            .collect())
    }
}

/// Combine two partial snapshots (e.g. the graph half and the anomaly half)
/// into one. Lists are concatenated, the active project prefers the first
/// present one, the capture time is the later of the two, and each coverage
/// flag is the OR of the inputs. This is how the daemon assembles the
/// per-source snapshots into the picture the explanation reasons over.
pub fn merge_snapshots(a: SystemSnapshot, b: SystemSnapshot) -> SystemSnapshot {
    let mut files = a.files;
    files.extend(b.files);
    let mut processes = a.processes;
    processes.extend(b.processes);
    let mut network = a.network;
    network.extend(b.network);
    let mut anomalies = a.anomalies;
    anomalies.extend(b.anomalies);
    SystemSnapshot {
        captured_at_unix: a.captured_at_unix.max(b.captured_at_unix),
        files,
        processes,
        network,
        anomalies,
        active_project: a.active_project.or(b.active_project),
        coverage: Coverage {
            graph_context: a.coverage.graph_context || b.coverage.graph_context,
            live_processes: a.coverage.live_processes || b.coverage.live_processes,
            anomalies: a.coverage.anomalies || b.coverage.anomalies,
        },
    }
}

#[cfg(test)]
mod anomaly_tests {
    use super::*;

    struct MockAnomalies(Vec<Anomaly>);
    impl AnomalyReader for MockAnomalies {
        fn read_anomalies(&self) -> Result<Vec<Anomaly>, SnapshotError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn alert_kind_maps_to_the_typed_anomaly() {
        assert_eq!(
            anomaly_from_alert("novel_node_type", "x").kind,
            AnomalyKind::NovelNodeAccess
        );
        assert_eq!(
            anomaly_from_alert("undeclared_network_destination", "x").kind,
            AnomalyKind::UndeclaredNetworkDestination
        );
        // Anything else (rate spike, AI-action-without-interaction, tampering)
        // is kept, not dropped, as the catch-all bucket.
        assert_eq!(
            anomaly_from_alert("query_rate_spike", "x").kind,
            AnomalyKind::UnusualForContext
        );
        assert_eq!(anomaly_from_alert("anything", "the summary").description, "the summary");
    }

    #[test]
    fn anomaly_context_sets_only_its_coverage_flag() {
        let reader = MockAnomalies(vec![Anomaly {
            kind: AnomalyKind::UnusualForContext,
            description: "spike".into(),
        }]);
        let snap = anomaly_context(&reader, 99).unwrap();
        assert_eq!(snap.captured_at_unix, 99);
        assert_eq!(snap.anomalies.len(), 1);
        assert!(snap.coverage.anomalies);
        assert!(!snap.coverage.graph_context);
        assert!(!snap.coverage.live_processes);
        assert!(snap.files.is_empty());
    }

    #[test]
    fn file_reader_parses_alerts_and_maps_them() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("alerts.json");
        std::fs::write(
            &path,
            r#"{ "alerts": [
                { "kind": "novel_node_type", "summary": "new node type", "body": "", "critical": false, "ts_micros": 1 },
                { "kind": "query_rate_spike", "summary": "rate spike", "body": "", "critical": true, "ts_micros": 2 }
            ] }"#,
        )
        .unwrap();
        let got = FileAnomalyReader::new(path.to_string_lossy().into_owned())
            .read_anomalies()
            .unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, AnomalyKind::NovelNodeAccess);
        assert_eq!(got[0].description, "new node type");
        assert_eq!(got[1].kind, AnomalyKind::UnusualForContext);
    }

    #[test]
    fn file_reader_treats_missing_or_malformed_as_no_anomalies() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("none.json");
        assert!(FileAnomalyReader::new(missing.to_string_lossy().into_owned())
            .read_anomalies()
            .unwrap()
            .is_empty());

        let bad = tmp.path().join("bad.json");
        std::fs::write(&bad, b"{ not json").unwrap();
        assert!(FileAnomalyReader::new(bad.to_string_lossy().into_owned())
            .read_anomalies()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn merge_concatenates_and_ors_coverage() {
        let graph = SystemSnapshot {
            captured_at_unix: 10,
            active_project: Some(ProjectContext { name: "p".into(), file_count: 3 }),
            coverage: Coverage { graph_context: true, live_processes: false, anomalies: false },
            ..Default::default()
        };
        let anomalies = SystemSnapshot {
            captured_at_unix: 20,
            anomalies: vec![Anomaly { kind: AnomalyKind::UnusualForContext, description: "x".into() }],
            coverage: Coverage { graph_context: false, live_processes: false, anomalies: true },
            ..Default::default()
        };
        let merged = merge_snapshots(graph, anomalies);
        assert_eq!(merged.captured_at_unix, 20);
        assert!(merged.active_project.is_some());
        assert_eq!(merged.anomalies.len(), 1);
        assert!(merged.coverage.graph_context);
        assert!(merged.coverage.anomalies);
        assert!(!merged.coverage.live_processes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A mock reader that returns canned rows per query, matched by a
    /// substring of the Cypher so a test can distinguish the file query
    /// from the project query.
    struct MockReader {
        responses: Vec<(&'static str, Vec<HashMap<String, Value>>)>,
        seen: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl GraphReader for MockReader {
        async fn query_rows(
            &self,
            cypher: &str,
        ) -> Result<Vec<HashMap<String, Value>>, SnapshotError> {
            self.seen.lock().unwrap().push(cypher.to_string());
            for (needle, rows) in &self.responses {
                if cypher.contains(needle) {
                    return Ok(rows.clone());
                }
            }
            Ok(vec![])
        }
    }

    fn row(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn required_labels_match_what_the_queries_read() {
        // Drift guard: every declared required label must actually
        // appear in one of the queries, so the scope check the daemon
        // runs against this list cannot silently under-cover a label the
        // queries read.
        let files = recent_files_query(0);
        let combined = format!("{files} {ACTIVE_PROJECT_QUERY}");
        for label in REQUIRED_GRAPH_LABELS {
            assert!(
                combined.contains(label),
                "required label {label} is not referenced by any query"
            );
        }
    }

    #[test]
    fn str_cell_treats_null_and_empty_as_absent() {
        let r = row(&[
            ("a", Value::String("x".into())),
            ("b", Value::Null),
            ("c", Value::String("".into())),
        ]);
        assert_eq!(str_cell(&r, "a"), Some("x".to_string()));
        assert_eq!(str_cell(&r, "b"), None);
        assert_eq!(str_cell(&r, "c"), None);
        assert_eq!(str_cell(&r, "missing"), None);
    }

    #[test]
    fn u64_cell_accepts_signed_and_unsigned_and_floors_negatives() {
        assert_eq!(u64_cell(&row(&[("n", Value::from(5u64))]), "n"), 5);
        assert_eq!(u64_cell(&row(&[("n", Value::from(7i64))]), "n"), 7);
        assert_eq!(u64_cell(&row(&[("n", Value::from(-3i64))]), "n"), 0);
        assert_eq!(u64_cell(&row(&[]), "n"), 0);
    }

    #[test]
    fn parse_files_skips_rows_missing_path_or_app() {
        let rows = vec![
            row(&[
                ("path", Value::String("/a".into())),
                ("app", Value::String("nvim".into())),
                ("project", Value::String("lun".into())),
            ]),
            // Missing app -> skipped.
            row(&[("path", Value::String("/b".into())), ("project", Value::Null)]),
            // Project NULL -> kept, no project.
            row(&[
                ("path", Value::String("/c".into())),
                ("app", Value::String("bash".into())),
                ("project", Value::Null),
            ]),
        ];
        let files = parse_files(&rows);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].project.as_deref(), Some("lun"));
        assert_eq!(files[1].path, "/c");
        assert_eq!(files[1].project, None);
    }

    #[test]
    fn parse_active_project_reads_name_and_count() {
        let rows = vec![row(&[
            ("name", Value::String("arlen".into())),
            ("file_count", Value::from(42i64)),
        ])];
        let p = parse_active_project(&rows).unwrap();
        assert_eq!(p.name, "arlen");
        assert_eq!(p.file_count, 42);
        assert_eq!(parse_active_project(&[]), None);
    }

    #[tokio::test]
    async fn graph_context_fills_files_and_project_only() {
        let reader = MockReader {
            responses: vec![
                (
                    "ACCESSED_BY",
                    vec![row(&[
                        ("path", Value::String("/p/main.rs".into())),
                        ("app", Value::String("nvim".into())),
                        ("project", Value::String("arlen".into())),
                    ])],
                ),
                (
                    "WHERE p.promoted",
                    vec![row(&[
                        ("name", Value::String("arlen".into())),
                        ("file_count", Value::from(3i64)),
                    ])],
                ),
            ],
            seen: Mutex::new(vec![]),
        };
        let snap = graph_context(&reader, 1234).await.unwrap();
        assert_eq!(snap.captured_at_unix, 1234);
        assert_eq!(snap.files.len(), 1);
        assert_eq!(snap.active_project.as_ref().unwrap().name, "arlen");
        // The graph source never fabricates the live-moment fields, and
        // marks only the graph-context coverage.
        assert!(snap.processes.is_empty());
        assert!(snap.network.is_empty());
        assert!(snap.anomalies.is_empty());
        assert!(snap.coverage.graph_context);
        assert!(!snap.coverage.live_processes);
        assert!(!snap.coverage.anomalies);
        assert!(!snap.coverage.is_complete());
    }

    #[tokio::test]
    async fn graph_context_queries_apply_recency_and_active_status() {
        let reader = MockReader {
            responses: vec![],
            seen: Mutex::new(vec![]),
        };
        // now = 10_000 s; window 6h => cutoff = (10_000 - 21_600) s, floored
        // at 0 micros (clock before the window start).
        let _ = graph_context(&reader, 10_000).await.unwrap();
        let seen = reader.seen.lock().unwrap().clone();
        let files_q = seen.iter().find(|q| q.contains("ACCESSED_BY")).unwrap();
        assert!(files_q.contains("f.last_accessed >= 0"), "{files_q}");
        let project_q = seen.iter().find(|q| q.contains("p.promoted")).unwrap();
        assert!(project_q.contains("p.status = 'active'"), "{project_q}");
        assert!(project_q.contains("count(DISTINCT f)"), "{project_q}");
    }

    #[tokio::test]
    async fn recency_cutoff_is_in_microseconds_past_the_window() {
        let reader = MockReader {
            responses: vec![],
            seen: Mutex::new(vec![]),
        };
        // now = 100_000 s, window 6h (21_600 s) => cutoff = 78_400 s in micros.
        let _ = graph_context(&reader, 100_000).await.unwrap();
        let seen = reader.seen.lock().unwrap().clone();
        let files_q = seen.iter().find(|q| q.contains("ACCESSED_BY")).unwrap();
        let expected = (100_000i64 - RECENT_WINDOW_SECS) * 1_000_000;
        assert!(files_q.contains(&format!("f.last_accessed >= {expected}")), "{files_q}");
    }

    #[tokio::test]
    async fn graph_context_propagates_a_read_error() {
        struct Failing;
        #[async_trait]
        impl GraphReader for Failing {
            async fn query_rows(
                &self,
                _: &str,
            ) -> Result<Vec<HashMap<String, Value>>, SnapshotError> {
                Err(SnapshotError::GraphRead("socket down".into()))
            }
        }
        let err = graph_context(&Failing, 0).await.unwrap_err();
        assert!(matches!(err, SnapshotError::GraphRead(_)));
    }
}
