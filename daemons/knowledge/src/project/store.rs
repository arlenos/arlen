//! Project CRUD and `PART_OF` edge operations against the Ladybug graph.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use std::sync::Arc;

use crate::drift::DeviceClock;
use crate::graph::{CellValue, GraphHandle, RowSet};
use crate::time::{dt_to_micros, micros_to_dt};
use crate::utils::{content_merge_key, escape_cypher};

// ── Types ───────────────────────────────────────────────────────────────

/// The namespace an inferred project's id is derived in. Fixed: changing it
/// renames every inferred project on every machine.
const INFERRED_NAMESPACE: Uuid = Uuid::from_u128(0x6a1f_3c2e_8b4d_4e6a_9c7b_2d5f_0e8a_1b3c);

/// An inferred project's id: a version-5 UUID of its root path, trailing slashes dropped.
///
/// It used to be random (`now_v7`), so a graph rebuilt by re-scanning gave every
/// inferred project a new id, and the memberships the journal records, which name
/// project ids, pointed at nothing. Derived from the root, detection and record
/// agree on the id however often the project is found again.
pub fn inferred_id(root_path: &str) -> Uuid {
    let root = root_path.trim_end_matches('/');
    let root = if root.is_empty() { "/" } else { root };
    Uuid::new_v5(&INFERRED_NAMESPACE, root.as_bytes())
}

/// Project status in the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectStatus {
    /// Project is active and visible.
    Active,
    /// Project has been archived (directory removed or .project deleted).
    Archived,
}

/// Outcome of a prune-or-archive validation pass on a single project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PruneOutcome {
    /// Project `root_path` exists on disk; nothing changed.
    Alive,
    /// Project was already archived before this pass; nothing changed.
    /// Treated as `Alive` for counting purposes — it is not a fresh
    /// transition.
    AlreadyArchived,
    /// Inferred project's `root_path` was missing; node was DETACH-DELETEd.
    Pruned,
    /// Explicit project's `root_path` was missing; status flipped to
    /// archived, history preserved.
    Archived,
}

/// Counts from a bulk prune pass over all active projects.
#[derive(Debug, Default, Clone, Copy)]
pub struct PruneStats {
    /// Projects whose `root_path` was found on disk (kept untouched).
    pub alive: usize,
    /// Inferred projects whose `root_path` vanished (deleted from graph).
    pub pruned: usize,
    /// Explicit projects whose `root_path` vanished (archived).
    pub archived: usize,
    /// Per-project failures during validation (logged, sweep continues).
    pub errors: usize,
}

impl ProjectStatus {
    /// Status as stored in the graph.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }

    /// Parse status from graph value.
    //
    // Deliberately not `FromStr`: that trait is fallible and this cannot fail -
    // an unknown value from the graph reads as Active rather than an error,
    // because a project whose status column drifted is still a project.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "archived" => Self::Archived,
            _ => Self::Active,
        }
    }
}

/// A project entity in the Knowledge Graph.
#[derive(Debug, Clone)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub root_path: String,
    pub accent_color: String,
    pub icon: String,
    pub status: ProjectStatus,
    pub created_at: DateTime<Utc>,
    pub last_accessed: Option<DateTime<Utc>>,
    pub inferred: bool,
    pub confidence: u8,
    pub promoted: bool,
    pub archived_at: Option<DateTime<Utc>>,
    /// Transaction-time close stamp (§4.9): `Some` once the project is archived
    /// (the system stopped believing it active), `None` while live. This is the
    /// one tombstone the bi-temporal liveness predicate reads.
    pub expired_at: Option<DateTime<Utc>>,
}

impl Project {
    /// Create a new inferred project from auto-detection.
    pub fn new_inferred(name: String, root_path: String, confidence: u8) -> Self {
        Self {
            id: inferred_id(&root_path),
            name,
            description: String::new(),
            root_path,
            accent_color: String::new(),
            icon: String::new(),
            status: ProjectStatus::Active,
            created_at: Utc::now(),
            last_accessed: None,
            inferred: true,
            confidence,
            promoted: false,
            archived_at: None,
            expired_at: None,
        }
    }

    /// Create a new explicit project from a .project file.
    pub fn new_explicit(id: Uuid, name: String, root_path: String) -> Self {
        Self {
            id,
            name,
            description: String::new(),
            root_path,
            accent_color: String::new(),
            icon: String::new(),
            status: ProjectStatus::Active,
            created_at: Utc::now(),
            last_accessed: None,
            inferred: false,
            confidence: 100,
            promoted: true,
            archived_at: None,
            expired_at: None,
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Parse a Project from a `RowSet` where columns match the SELECT order.
fn parse_project(rs: &RowSet, row_idx: usize) -> Option<Project> {
    let row = rs.rows.get(row_idx)?;
    if row.len() < 14 {
        return None;
    }
    let col = |name: &str| -> usize {
        rs.columns.iter().position(|c| c == name).unwrap_or(usize::MAX)
    };
    let s = |i: usize| -> String {
        row.get(i).map(|v| v.as_str().to_string()).unwrap_or_default()
    };
    let i = |i: usize| -> i64 {
        row.get(i).map(|v| v.as_i64()).unwrap_or(0)
    };
    let b = |i: usize| -> bool {
        row.get(i)
            .map(|v| match v {
                CellValue::Bool(b) => *b,
                CellValue::Int64(n) => *n != 0,
                _ => false,
            })
            .unwrap_or(false)
    };

    let id_idx = col("p.id");
    let id_str = s(id_idx);
    let id = Uuid::parse_str(&id_str).ok()?;
    let created_at_ms = i(col("p.created_at"));

    Some(Project {
        id,
        name: s(col("p.name")),
        description: s(col("p.description")),
        root_path: s(col("p.root_path")),
        accent_color: s(col("p.accent_color")),
        icon: s(col("p.icon")),
        status: ProjectStatus::from_str(&s(col("p.status"))),
        created_at: micros_to_dt(created_at_ms).unwrap_or_else(Utc::now),
        last_accessed: micros_to_dt(i(col("p.last_accessed"))),
        inferred: b(col("p.inferred")),
        confidence: i(col("p.confidence")) as u8,
        promoted: b(col("p.promoted")),
        archived_at: micros_to_dt(i(col("p.archived_at"))),
        expired_at: micros_to_dt(i(col("p.expired_at"))),
    })
}

/// Column list for SELECT queries.
const PROJECT_COLUMNS: &str = "p.id, p.name, p.description, p.root_path, \
    p.accent_color, p.icon, p.status, p.created_at, p.last_accessed, \
    p.inferred, p.confidence, p.promoted, p.archived_at, p.expired_at";

/// A fact journal for a test's store, in the test's own directory.
#[cfg(test)]
pub(crate) async fn test_pool(dir: &std::path::Path) -> sqlx::SqlitePool {
    crate::db::open(dir.join("journal.db").to_str().unwrap()).await.unwrap()
}

// ── ProjectStore ────────────────────────────────────────────────────────

/// Store for Project CRUD and `PART_OF` edge operations.
pub struct ProjectStore {
    graph: GraphHandle,
    /// The fact journal every write here goes through (D11): a project, its
    /// promotion and its memberships are the record, Ladybug is their projection.
    pool: sqlx::SqlitePool,
    /// The device-wide merge clock. When set, `link_file` stamps the HLC and
    /// device id on a new membership so a future cross-device merge can order
    /// promoted edges (graph-drift.md §2). Left `None` in tests and non-writing
    /// constructions, which then create unstamped edges (NULL merge columns,
    /// consistent with the columns' no-backfill rule).
    clock: Option<Arc<DeviceClock>>,
}

impl ProjectStore {
    /// Create a new `ProjectStore` with no merge clock (unstamped writes).
    pub fn new(graph: GraphHandle, pool: sqlx::SqlitePool) -> Self {
        Self { graph, pool, clock: None }
    }

    /// Record `facts` and bring the graph up to date. The provenance is `graph`:
    /// the system observed these, nobody asserted them.
    async fn journal(&self, facts: Vec<crate::journal::Fact>) -> Result<()> {
        let origin = crate::journal::Origin {
            provenance: crate::provenance::Provenance::Graph.as_key().to_string(),
            valid_time_source: String::new(),
            event_id: None,
        };
        crate::journal::commit_and_project(&self.pool, &self.graph, facts, &origin).await
    }

    /// Attach the device-wide merge clock so `link_file` stamps the ordering
    /// columns. The production daemon builds one `Arc<DeviceClock>` and shares
    /// it across every writer so their HLCs are comparable.
    pub fn with_clock(mut self, clock: Arc<DeviceClock>) -> Self {
        self.clock = Some(clock);
        self
    }

    /// Insert a new project node.
    pub async fn create(&self, project: &Project) -> Result<()> {
        let status = project.status.as_str();
        let created = dt_to_micros(&project.created_at);
        let accessed = project.last_accessed.map(|d| dt_to_micros(&d)).unwrap_or(0);
        let inferred = project.inferred;
        let confidence = project.confidence as i64;
        let promoted = project.promoted;

        // Check for duplicate root_path first.
        let check = self.get_by_root_path(&project.root_path).await?;
        if let Some(existing) = check {
            if existing.id != project.id {
                return Err(anyhow!(
                    "project with root_path '{}' already exists (id: {})",
                    project.root_path,
                    existing.id
                ));
            }
        }

        // Through the journal: a node upsert, so finding an inferred project
        // again at the same root (same derived id) reopens the one that was
        // closed rather than failing on a duplicate key.
        use serde_json::json;
        self.journal(vec![crate::journal::Fact::Node {
            label: "Project".into(),
            id: project.id.to_string(),
            props: [
                ("name", json!(project.name)),
                ("description", json!(project.description)),
                ("root_path", json!(project.root_path)),
                ("accent_color", json!(project.accent_color)),
                ("icon", json!(project.icon)),
                ("status", json!(status)),
                ("last_accessed", json!(accessed)),
                ("inferred", json!(inferred)),
                ("confidence", json!(confidence)),
                ("promoted", json!(promoted)),
                ("archived_at", json!(0)),
                ("expired_at", serde_json::Value::Null),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            on_create: [("created_at".to_string(), json!(created))].into_iter().collect(),
        }])
        .await
    }

    /// Get a project by its UUID.
    pub async fn get_by_id(&self, id: Uuid) -> Result<Option<Project>> {
        let id_esc = escape_cypher(&id.to_string());
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH (p:Project {{id: '{id_esc}'}}) RETURN {PROJECT_COLUMNS}"
            ))
            .await?;
        if rs.rows.is_empty() {
            return Ok(None);
        }
        Ok(parse_project(&rs, 0))
    }

    /// Get a project by its exact `root_path`.
    pub async fn get_by_root_path(&self, path: &str) -> Result<Option<Project>> {
        let path_esc = escape_cypher(path);
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH (p:Project {{root_path: '{path_esc}'}}) WHERE p.expired_at IS NULL \
                 RETURN {PROJECT_COLUMNS}"
            ))
            .await?;
        if rs.rows.is_empty() {
            return Ok(None);
        }
        Ok(parse_project(&rs, 0))
    }

    /// Find the project whose `root_path` is the longest prefix of `file_path`.
    ///
    /// This implements the "nearest ancestor" rule: if a file lives inside
    /// nested projects, the innermost (longest path) project wins.
    pub async fn find_by_path_prefix(&self, file_path: &str) -> Result<Option<Project>> {
        let fp_esc = escape_cypher(file_path);
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH (p:Project)
                 WHERE p.status = 'active'
                   AND starts_with('{fp_esc}', p.root_path)
                 RETURN {PROJECT_COLUMNS}
                 ORDER BY size(p.root_path) DESC
                 LIMIT 1"
            ))
            .await?;
        if rs.rows.is_empty() {
            return Ok(None);
        }
        Ok(parse_project(&rs, 0))
    }

    /// How many active projects live strictly INSIDE `root`.
    ///
    /// The test behind "a container of projects is not a project"
    /// (`project-system.md`). Strictly inside: a project AT `root` is not one of
    /// its own contents, which is why the prefix carries the separator.
    pub async fn count_projects_under(&self, root: &str) -> Result<usize> {
        let prefix = escape_cypher(&format!("{}/", root.trim_end_matches('/')));
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH (p:Project)
                 WHERE p.status = 'active'
                   AND starts_with(p.root_path, '{prefix}')
                 RETURN count(p) AS n"
            ))
            .await?;
        Ok(rs.rows.first().and_then(|r| r.first()).map(|c| c.as_i64()).unwrap_or(0).max(0) as usize)
    }

    /// List all active projects.
    pub async fn list_active(&self) -> Result<Vec<Project>> {
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH (p:Project) WHERE p.status = 'active' RETURN {PROJECT_COLUMNS}"
            ))
            .await?;
        Ok((0..rs.rows.len())
            .filter_map(|i| parse_project(&rs, i))
            .collect())
    }

    /// List promoted projects (visible in Waypointer and Focus Mode).
    pub async fn list_promoted(&self) -> Result<Vec<Project>> {
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH (p:Project)
                 WHERE p.promoted = true AND p.status = 'active'
                 RETURN {PROJECT_COLUMNS}"
            ))
            .await?;
        Ok((0..rs.rows.len())
            .filter_map(|i| parse_project(&rs, i))
            .collect())
    }

    /// Update a project's mutable fields.
    pub async fn update(&self, project: &Project) -> Result<()> {
        use serde_json::json;
        self.journal(vec![crate::journal::Fact::node(
            "Project",
            &project.id.to_string(),
            &[
                ("name", json!(project.name)),
                ("description", json!(project.description)),
                ("accent_color", json!(project.accent_color)),
                ("icon", json!(project.icon)),
                ("status", json!(project.status.as_str())),
                ("inferred", json!(project.inferred)),
                ("confidence", json!(project.confidence as i64)),
                ("promoted", json!(project.promoted)),
            ],
        )])
        .await
    }

    /// Archive a project (soft delete).
    pub async fn archive(&self, id: Uuid) -> Result<()> {
        let now = crate::time::now().0;
        // Archiving is a transaction-time close (§4.9): `expired_at` is the one
        // tombstone the bi-temporal model reads (a live project is
        // `expired_at IS NULL`); `status`/`archived_at` stay as denormalised read
        // filters set alongside it.
        self.journal(vec![Self::closed(id, now)]).await
    }

    /// The node fact that closes a project at `now`.
    fn closed(id: Uuid, now: i64) -> crate::journal::Fact {
        use serde_json::json;
        crate::journal::Fact::node(
            "Project",
            &id.to_string(),
            &[("status", json!("archived")), ("archived_at", json!(now)), ("expired_at", json!(now))],
        )
    }

    /// Withdraw a project: close it and every live membership into it.
    ///
    /// This was a `DETACH DELETE`, and the record has no delete: an inferred
    /// project that turned out wrong is now closed like an archived one, its
    /// memberships closed with it. The reads already show only active projects,
    /// so nothing pollutes Waypointer, and a rebuild from the journal arrives at
    /// the same closed project instead of resurrecting it. Found again at the same
    /// root, it gets the same id back and `create` reopens it.
    pub async fn retract(&self, id: Uuid) -> Result<()> {
        let now = crate::time::now().0;
        self.journal(vec![
            crate::journal::Fact::CloseTo {
                rel: "FILE_PART_OF".into(),
                to: ("Project".into(), id.to_string()),
                at: now,
            },
            Self::closed(id, now),
        ])
        .await
    }

    /// Promote a project (make visible in Waypointer).
    pub async fn promote(&self, id: Uuid) -> Result<()> {
        // Journaled above all the rest: a person's promotion of an inferred
        // project is recovered from nowhere else.
        self.journal(vec![crate::journal::Fact::node(
            "Project",
            &id.to_string(),
            &[("promoted", serde_json::json!(true))],
        )])
        .await
    }

    /// Refresh `last_accessed`: the latest access of any file that is a live
    /// member of the project.
    ///
    /// A projection aggregate, not a fact, so it is not journaled: it is computed
    /// from the files' own `last_accessed`, which their open events record. It
    /// used to be the clock at the moment of the call, written on every open, the
    /// busiest write the store made. After a rebuild a project's value returns
    /// with the next open inside it.
    pub async fn touch(&self, id: Uuid) -> Result<()> {
        let id_esc = escape_cypher(&id.to_string());
        self.graph
            .write(format!(
                "MATCH (p:Project {{id: '{id_esc}'}}) \
                 OPTIONAL MATCH (f:File)-[r:FILE_PART_OF]->(p) \
                   WHERE r.invalid_at IS NULL AND r.expired_at IS NULL \
                 WITH p, max(f.last_accessed) AS latest \
                 SET p.last_accessed = CASE WHEN latest IS NULL THEN p.last_accessed ELSE latest END"
            ))
            .await?;
        Ok(())
    }

    /// Check if the project's `root_path` still exists on disk.
    pub async fn validate_path(&self, id: Uuid) -> Result<bool> {
        if let Some(project) = self.get_by_id(id).await? {
            Ok(std::path::Path::new(&project.root_path).exists())
        } else {
            Ok(false)
        }
    }

    /// Validate a project's `root_path` on disk and prune (inferred) or
    /// archive (explicit) when the directory is gone.
    ///
    /// Per `docs/architecture/project-system.md` §Validation on Access —
    /// the spec says no periodic polling; instead we validate when a
    /// project is touched (Waypointer list, Focus Mode activation,
    /// daemon startup). One `stat()` per project is cheap.
    ///
    /// Inferred projects are deleted outright because their existence
    /// was a heuristic guess — keeping a graveyard of dead inferences
    /// would only pollute Waypointer's project list. Explicit projects
    /// (the user wrote a `.project` file) get archived so the activity
    /// history survives even when the working tree is moved or deleted.
    pub async fn prune_or_archive(&self, id: Uuid) -> Result<PruneOutcome> {
        let project = match self.get_by_id(id).await? {
            Some(p) => p,
            None => return Ok(PruneOutcome::Alive),
        };

        if project.status == ProjectStatus::Archived {
            return Ok(PruneOutcome::AlreadyArchived);
        }

        if std::path::Path::new(&project.root_path).exists() {
            return Ok(PruneOutcome::Alive);
        }

        if project.inferred {
            self.retract(id).await?;
            Ok(PruneOutcome::Pruned)
        } else {
            self.archive(id).await?;
            Ok(PruneOutcome::Archived)
        }
    }

    /// Walk every active project and validate-or-prune each. Used as a
    /// startup pass so the daemon never serves a project whose root has
    /// been gone for hours. Errors on individual projects are logged
    /// and counted in `PruneStats.errors` but do not abort the sweep.
    pub async fn prune_dead_projects(&self) -> Result<PruneStats> {
        let projects = self.list_active().await?;
        let mut stats = PruneStats::default();
        for project in projects {
            match self.prune_or_archive(project.id).await {
                Ok(PruneOutcome::Alive) | Ok(PruneOutcome::AlreadyArchived) => stats.alive += 1,
                Ok(PruneOutcome::Pruned) => stats.pruned += 1,
                Ok(PruneOutcome::Archived) => stats.archived += 1,
                Err(err) => {
                    tracing::warn!(
                        project_id = %project.id,
                        error = %err,
                        "prune_or_archive failed; leaving project as-is"
                    );
                    stats.errors += 1;
                }
            }
        }
        Ok(stats)
    }

    // ── PART_OF Edge Operations ─────────────────────────────────────────

    /// Create a `FILE_PART_OF` edge from a File node to a Project node.
    pub async fn link_file(&self, file_id: &str, project_id: Uuid) -> Result<()> {
        let pid_str = project_id.to_string();
        let fid = escape_cypher(file_id);
        let pid = escape_cypher(&pid_str);
        // The content-addressed merge key (GD-R1), from the RAW ids so it matches
        // the agent write path's key for the same membership (both use the bare
        // File/Project labels and the same node ids), letting a future merge
        // dedup a promoted edge and an agent-written one to one identity. `ON
        // CREATE SET` stamps only a newly-created edge (no backfill of an
        // existing one, consistent with the bitemporal stamps' no-backfill rule).
        let merge_key = content_merge_key("File", file_id, "FILE_PART_OF", "Project", &pid_str);
        // The bitemporal stamps, which this write has been leaving NULL. The
        // columns have existed since the KG work and the agent's write path fills
        // them (`persist_file_part_of`), but promotion - which creates almost
        // every membership in a real graph - set only the merge key. Measured 16
        // August: every `FILE_PART_OF` in a live graph carried `valid_at`,
        // `invalid_at` and `created_at` NULL, so nothing could ask what this file
        // belonged to at a point in time; the editor's as-of control is disabled
        // for exactly this reason.
        //
        // Liveness reads are unaffected: they test `invalid_at IS NULL AND
        // expired_at IS NULL`, and those stay unset (NULL) on a fresh edge, which
        // is the open interval. This only ADDS the start of it. `origin` is
        // `graph` - the system observed this, no one asserted it - matching
        // `provenance::Provenance::Graph`.
        let stamped_at = crate::time::now().0;
        // The enum, not the literal `'graph'`. `Provenance::from_key` fails closed
        // on an unknown key, so a rename here would not break loudly - it would
        // make every promoted edge's provenance unreadable, and the governance
        // gate refuses a write driven by a fact of unknown origin. The one place
        // that defines the key should be the one place that spells it.
        let origin = crate::provenance::Provenance::Graph.as_key();
        // The cross-device ordering stamp (GD-R5), only when a merge clock is
        // attached. The device id is a UUID by construction (no quote or
        // backslash), safe to interpolate like the hex `merge_key`.
        use serde_json::json;
        let mut props: std::collections::BTreeMap<String, serde_json::Value> = [
            ("merge_key", json!(merge_key)),
            ("origin", json!(origin)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        if let Some(clock) = &self.clock {
            // Epoch micros are always positive, so the i64 -> u64 is lossless;
            // the value fits the INT64 column when read back.
            let h = clock.stamp(crate::time::now().0 as u64);
            props.insert("hlc_physical".into(), json!(h.physical as i64));
            props.insert("hlc_logical".into(), json!(h.logical as i64));
            props.insert("device_id".into(), json!(clock.device_id().to_string()));
        }
        // Idempotent like the MERGE it replaces: a live membership to this
        // project already holds, so there is nothing to record. A closed one does
        // not count, and a new link then appends instead of reopening it.
        let live = self
            .graph
            .query_rows(format!(
                "MATCH (f:File {{id: '{fid}'}})-[r:FILE_PART_OF]->(p:Project {{id: '{pid}'}}) \
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL RETURN count(*) AS n"
            ))
            .await?;
        if live.rows.first().and_then(|r| r.first()).map(|c| c.as_i64()).unwrap_or(0) > 0 {
            return Ok(());
        }
        // An op id of its own per assertion, so the journal's upsert-by-op_id can
        // never land on an earlier, closed membership and clear its stamps.
        self.journal(vec![crate::journal::Fact::Edge {
            rel: "FILE_PART_OF".into(),
            from: ("File".into(), file_id.to_string()),
            to: ("Project".into(), pid_str.clone()),
            op_id: Some(format!("link:{merge_key}:{stamped_at}")),
            stamps: crate::journal::Stamps {
                valid_at: Some(stamped_at),
                created_at: Some(stamped_at),
                ..Default::default()
            },
            props,
        }])
        .await
    }

    /// Check if a file is already linked to a project, by a live OR a closed
    /// membership. Closed counts on purpose: a file the agent moved to another
    /// project carries a closed edge here, and promotion must not pull it back on
    /// its next open.
    pub async fn is_file_linked(&self, file_id: &str, project_id: Uuid) -> Result<bool> {
        let fid = escape_cypher(file_id);
        let pid = escape_cypher(&project_id.to_string());
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH (f:File {{id: '{fid}'}})-[:FILE_PART_OF]->(p:Project {{id: '{pid}'}})
                 RETURN count(*) AS cnt"
            ))
            .await?;
        let count = rs
            .rows
            .first()
            .and_then(|r| r.first())
            .map(|v| v.as_i64())
            .unwrap_or(0);
        Ok(count > 0)
    }

    /// Get all file paths in a project.
    pub async fn get_project_files(&self, project_id: Uuid) -> Result<Vec<String>> {
        let pid = escape_cypher(&project_id.to_string());
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project {{id: '{pid}'}})
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL
                 RETURN f.path"
            ))
            .await?;
        Ok(rs
            .rows
            .iter()
            .filter_map(|r| r.first().map(|v| v.as_str().to_string()))
            .collect())
    }

    /// Every distinct `origin` on the project's LIVE memberships.
    ///
    /// The question this answers is "did anybody but the observation pipeline
    /// put a file in here". `link_file` stamps `graph` (the system watched it
    /// happen), the agent write path stamps `agent`, and an assertion a person
    /// made is `user`, so a project whose origins are exactly `["graph"]` holds
    /// nothing anyone chose to put there. Closed memberships are excluded: a
    /// file somebody once added and then removed is not a member now.
    ///
    /// A NULL origin is reported as the empty string rather than skipped.
    /// Promotion left the column unset for as long as it existed, so a graph
    /// with history in it has NULLs, and silently reading those as `graph`
    /// would be inventing a provenance the row does not carry.
    pub async fn member_origins(&self, project_id: Uuid) -> Result<Vec<String>> {
        let pid = escape_cypher(&project_id.to_string());
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH ()-[r:FILE_PART_OF]->(:Project {{id: '{pid}'}})
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL
                 RETURN DISTINCT r.origin AS origin"
            ))
            .await?;
        Ok(rs
            .rows
            .iter()
            .filter_map(|r| r.first().map(|v| v.as_str().to_string()))
            .collect())
    }

    /// Close every live `FILE_PART_OF` membership pointing to a project.
    pub async fn unlink_all_files(&self, project_id: Uuid) -> Result<()> {
        // A close, not a delete: the memberships stay as history (§4.7).
        self.journal(vec![crate::journal::Fact::CloseTo {
            rel: "FILE_PART_OF".into(),
            to: ("Project".into(), project_id.to_string()),
            at: crate::time::now().0,
        }])
        .await
    }

    /// Count distinct files linked to a project that were accessed by an
    /// app active in the given session.
    pub async fn count_session_files(
        &self,
        session_id: &str,
        project_id: Uuid,
    ) -> Result<usize> {
        let pid = escape_cypher(&project_id.to_string());
        let sid = escape_cypher(session_id);
        let rs = self
            .graph
            .query_rows(format!(
                "MATCH (f:File)-[r:FILE_PART_OF]->(p:Project {{id: '{pid}'}})
                 WHERE r.invalid_at IS NULL AND r.expired_at IS NULL
                 MATCH (f)-[:ACCESSED_BY]->(a:App)-[:ACTIVE_IN]->(s:Session {{id: '{sid}'}})
                 RETURN count(DISTINCT f) AS cnt"
            ))
            .await?;
        let count = rs
            .rows
            .first()
            .and_then(|r| r.first())
            .map(|v| v.as_i64())
            .unwrap_or(0);
        Ok(count as usize)
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// `as_str` is not a display helper, it is the storage format: these exact
    /// strings sit in `p.status` on every Project node ever written. Renaming a
    /// variant would quietly change them and orphan every existing row, which
    /// reads back through the `_ => Active` arm below as a resurrected project
    /// rather than as an error. Pinned literally so that rename has to happen
    /// here first, where the migration question is unavoidable.
    #[test]
    fn the_stored_status_strings_are_the_storage_format() {
        assert_eq!(ProjectStatus::Active.as_str(), "active");
        assert_eq!(ProjectStatus::Archived.as_str(), "archived");
        for status in [ProjectStatus::Active, ProjectStatus::Archived] {
            assert_eq!(
                ProjectStatus::from_str(status.as_str()),
                status,
                "{status:?} does not survive a write and read back"
            );
        }
    }

    /// The parse is deliberately total and defaults to `Active`, which matters
    /// for rows written before the column existed. Worth pinning the direction:
    /// an unreadable status shows a project rather than hiding one.
    #[test]
    fn an_unrecognised_status_reads_as_active() {
        assert_eq!(ProjectStatus::from_str(""), ProjectStatus::Active);
        assert_eq!(ProjectStatus::from_str("Archived"), ProjectStatus::Active);
    }

    async fn setup() -> (ProjectStore, TempDir) {
        let tmp = TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        // Small delay for schema creation.
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
        let pool = test_pool(tmp.path()).await;
        (ProjectStore::new(graph, pool), tmp)
    }

    #[tokio::test]
    async fn test_create_and_get_by_id() {
        let (store, _tmp) = setup().await;
        let p = Project::new_inferred("my-app".into(), "/home/user/my-app".into(), 90);
        store.create(&p).await.unwrap();

        let got = store.get_by_id(p.id).await.unwrap();
        assert!(got.is_some());
        let got = got.unwrap();
        assert_eq!(got.name, "my-app");
        assert_eq!(got.root_path, "/home/user/my-app");
        assert!(got.inferred);
        assert_eq!(got.confidence, 90);
        assert!(!got.promoted);
        assert_eq!(got.status, ProjectStatus::Active);
    }

    #[tokio::test]
    async fn test_get_by_root_path() {
        let (store, _tmp) = setup().await;
        let p = Project::new_inferred("app".into(), "/home/user/app".into(), 80);
        store.create(&p).await.unwrap();

        let found = store.get_by_root_path("/home/user/app").await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, p.id);

        let missing = store.get_by_root_path("/home/user/other").await.unwrap();
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_find_by_path_prefix_nearest_ancestor() {
        let (store, _tmp) = setup().await;

        let parent = Project::new_inferred("monorepo".into(), "/home/user/mono".into(), 90);
        store.create(&parent).await.unwrap();

        let nested = Project::new_inferred(
            "app-a".into(),
            "/home/user/mono/packages/app-a".into(),
            100,
        );
        store.create(&nested).await.unwrap();

        // File inside nested -> nearest ancestor = nested
        let found = store
            .find_by_path_prefix("/home/user/mono/packages/app-a/src/main.rs")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "app-a");

        // File in parent but outside nested -> nearest = parent
        let found = store
            .find_by_path_prefix("/home/user/mono/docs/readme.md")
            .await
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "monorepo");

        // File outside everything -> None
        let found = store
            .find_by_path_prefix("/home/user/downloads/file.txt")
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_list_active() {
        let (store, _tmp) = setup().await;

        let a = Project::new_inferred("active".into(), "/a".into(), 90);
        let b = Project::new_inferred("to-archive".into(), "/b".into(), 90);
        store.create(&a).await.unwrap();
        store.create(&b).await.unwrap();
        store.archive(b.id).await.unwrap();

        let active = store.list_active().await.unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "active");
    }

    #[tokio::test]
    async fn test_list_promoted() {
        let (store, _tmp) = setup().await;

        let explicit = Project::new_explicit(Uuid::now_v7(), "explicit".into(), "/a".into());
        let inferred = Project::new_inferred("inferred".into(), "/b".into(), 80);
        store.create(&explicit).await.unwrap();
        store.create(&inferred).await.unwrap();

        let promoted = store.list_promoted().await.unwrap();
        assert_eq!(promoted.len(), 1);
        assert_eq!(promoted[0].name, "explicit");
    }

    #[tokio::test]
    async fn test_promote() {
        let (store, _tmp) = setup().await;

        let p = Project::new_inferred("test".into(), "/a".into(), 90);
        store.create(&p).await.unwrap();
        assert!(!p.promoted);

        store.promote(p.id).await.unwrap();

        let got = store.get_by_id(p.id).await.unwrap().unwrap();
        assert!(got.promoted);
    }

    #[tokio::test]
    async fn test_archive() {
        let (store, _tmp) = setup().await;

        let p = Project::new_inferred("test".into(), "/a".into(), 90);
        store.create(&p).await.unwrap();
        store.archive(p.id).await.unwrap();

        let got = store.get_by_id(p.id).await.unwrap().unwrap();
        assert_eq!(got.status, ProjectStatus::Archived);
        assert!(got.archived_at.is_some());
        // §4.9: archiving is a transaction-time close, so `expired_at` is set;
        // the bi-temporal liveness predicate reads this, not `status`.
        assert!(got.expired_at.is_some(), "archive sets the expired_at close stamp");
    }

    #[tokio::test]
    async fn a_fresh_project_is_live_with_no_expired_at() {
        let (store, _tmp) = setup().await;
        let p = Project::new_inferred("live".into(), "/b".into(), 90);
        store.create(&p).await.unwrap();
        let got = store.get_by_id(p.id).await.unwrap().unwrap();
        assert!(got.expired_at.is_none(), "a live project has a NULL expired_at");
    }

    #[tokio::test]
    async fn test_touch() {
        let (store, _tmp) = setup().await;

        let p = Project::new_inferred("test".into(), "/a".into(), 90);
        store.create(&p).await.unwrap();

        // `last_accessed` is the latest access of a live member file, not the
        // clock at the moment of the call.
        store
            .graph
            .write("CREATE (f:File {id: '/a/x.rs', path: '/a/x.rs', app_id: 't', last_accessed: 1780000000000000})".into())
            .await
            .unwrap();
        store.link_file("/a/x.rs", p.id).await.unwrap();
        store.touch(p.id).await.unwrap();

        let got = store.get_by_id(p.id).await.unwrap().unwrap();
        assert_eq!(got.last_accessed.map(|d| crate::time::dt_to_micros(&d)), Some(1_780_000_000_000_000));
    }

    #[tokio::test]
    async fn retract_closes_the_project_and_its_memberships() {
        let (store, _tmp) = setup().await;

        let p = Project::new_inferred("test".into(), "/a".into(), 90);
        store.create(&p).await.unwrap();
        store
            .graph
            .write("CREATE (f:File {id: '/a/x.rs', path: '/a/x.rs', app_id: 't', last_accessed: 0})".into())
            .await
            .unwrap();
        store.link_file("/a/x.rs", p.id).await.unwrap();
        store.retract(p.id).await.unwrap();

        // Closed, not gone: the node stays as history, out of every active read.
        let got = store.get_by_id(p.id).await.unwrap().expect("kept");
        assert_eq!(got.status, ProjectStatus::Archived);
        assert!(got.expired_at.is_some());
        assert!(store.list_active().await.unwrap().is_empty());
        assert!(store.get_project_files(p.id).await.unwrap().is_empty(), "memberships closed");
        assert!(store.get_by_root_path("/a").await.unwrap().is_none());

        // Found again at the same root: the same id, reopened.
        let again = Project::new_inferred("test".into(), "/a/".into(), 90);
        assert_eq!(again.id, p.id, "the id comes from the root");
        store.create(&again).await.unwrap();
        let back = store.get_by_id(p.id).await.unwrap().unwrap();
        assert_eq!(back.status, ProjectStatus::Active);
        assert!(back.expired_at.is_none());
    }

    /// What the ruling is for: a person's promotion of an inferred project, and
    /// its memberships, come back from the journal into a fresh graph.
    #[tokio::test]
    async fn a_promoted_inferred_project_survives_a_rebuild() {
        let tmp = TempDir::new().unwrap();
        let pool = test_pool(tmp.path()).await;
        let graph = crate::graph::spawn(tmp.path().join("g1").to_str().unwrap()).unwrap();
        let store = ProjectStore::new(graph.clone(), pool.clone());
        let file = "CREATE (f:File {id: '/w/a.rs', path: '/w/a.rs', app_id: 't', last_accessed: 0})";
        graph.write(file.into()).await.unwrap();
        let p = Project::new_inferred("w".into(), "/w".into(), 70);
        store.create(&p).await.unwrap();
        store.link_file("/w/a.rs", p.id).await.unwrap();
        store.promote(p.id).await.unwrap();

        let fresh = crate::graph::spawn(tmp.path().join("g2").to_str().unwrap()).unwrap();
        fresh.write(file.into()).await.unwrap();
        crate::journal::set_projected_seq(&pool, 0).await.unwrap();
        crate::journal::project_pending(&pool, &fresh).await.unwrap();
        let rebuilt = ProjectStore::new(fresh, pool);
        let got = rebuilt.get_by_id(p.id).await.unwrap().expect("the project is rebuilt");
        assert!(got.promoted, "and its promotion with it");
        assert_eq!(rebuilt.get_project_files(p.id).await.unwrap(), vec!["/w/a.rs".to_string()]);
    }

    #[tokio::test]
    async fn test_unique_root_path() {
        let (store, _tmp) = setup().await;

        // Two inferred projects at one root are one project: the id is the
        // root's. A different project claiming that root is refused.
        let p1 = Project::new_inferred("first".into(), "/same".into(), 90);
        assert_eq!(Project::new_inferred("second".into(), "/same".into(), 90).id, p1.id);
        store.create(&p1).await.unwrap();

        let p2 = Project::new_explicit(Uuid::now_v7(), "second".into(), "/same".into());
        let result = store.create(&p2).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update() {
        let (store, _tmp) = setup().await;

        let mut p = Project::new_inferred("old-name".into(), "/a".into(), 80);
        store.create(&p).await.unwrap();

        p.name = "new-name".into();
        p.description = "A description".into();
        p.accent_color = "#6366f1".into();
        p.confidence = 100;
        store.update(&p).await.unwrap();

        let got = store.get_by_id(p.id).await.unwrap().unwrap();
        assert_eq!(got.name, "new-name");
        assert_eq!(got.description, "A description");
        assert_eq!(got.accent_color, "#6366f1");
        assert_eq!(got.confidence, 100);
    }

    #[tokio::test]
    async fn test_link_file_and_check() {
        let (store, _tmp) = setup().await;

        let p = Project::new_inferred("test".into(), "/a".into(), 90);
        store.create(&p).await.unwrap();

        // Create a File node.
        let file_path = "/a/src/main.rs";
        store
            .graph
            .write(format!(
                "CREATE (f:File {{id: '{file_path}', path: '{file_path}', \
                 app_id: 'test', last_accessed: 0}})"
            ))
            .await
            .unwrap();

        // Link file to project.
        store.link_file(file_path, p.id).await.unwrap();
        assert!(store.is_file_linked(file_path, p.id).await.unwrap());

        // Get project files.
        let files = store.get_project_files(p.id).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], file_path);
    }

    #[tokio::test]
    async fn link_file_stamps_the_content_merge_key_matching_the_agent_path() {
        // GD-R1: the promotion pipeline stamps the SAME content merge key the
        // agent write path would for this membership, so a future cross-device
        // merge dedups a promoted edge and an agent-written one to one identity.
        let (store, _tmp) = setup().await;
        let p = Project::new_inferred("test".into(), "/a".into(), 90);
        store.create(&p).await.unwrap();
        let file_path = "/a/src/main.rs";
        store
            .graph
            .write(format!(
                "CREATE (f:File {{id: '{file_path}', path: '{file_path}', \
                 app_id: 'test', last_accessed: 0}})"
            ))
            .await
            .unwrap();
        store.link_file(file_path, p.id).await.unwrap();

        let row = store
            .graph
            .query_rows(format!(
                "MATCH (:File {{id: '{file_path}'}})-[r:FILE_PART_OF]->(:Project {{id: '{}'}}) \
                 RETURN r.merge_key AS mk",
                p.id
            ))
            .await
            .unwrap();
        let stamped = row.rows[0][0].as_str();
        // Same helper, same content tuple (bare labels + the same node ids) the
        // agent path uses, so the keys are equal across the two creation paths.
        let expected =
            content_merge_key("File", file_path, "FILE_PART_OF", "Project", &p.id.to_string());
        assert_eq!(stamped, expected, "the promoted edge carries the content merge key");
        assert_eq!(stamped.len(), 64, "the merge key is the fixed-length hex digest");
    }

    #[tokio::test]
    async fn link_file_opens_the_bitemporal_interval() {
        // These stamps are read by NOTHING today, which is exactly why they need
        // a test: promotion left them NULL for as long as the columns existed,
        // and no surface noticed because the liveness reads all test `IS NULL`
        // and a missing `valid_at` looks identical to an open one. The cost only
        // showed up as an as-of question nobody could answer.
        let (store, _tmp) = setup().await;
        let p = Project::new_inferred("test".into(), "/a".into(), 90);
        store.create(&p).await.unwrap();
        let file_path = "/a/src/main.rs";
        store
            .graph
            .write(format!(
                "CREATE (f:File {{id: '{file_path}', path: '{file_path}', \
                 app_id: 'test', last_accessed: 0}})"
            ))
            .await
            .unwrap();
        let before = crate::time::now().0;
        store.link_file(file_path, p.id).await.unwrap();

        let row = store
            .graph
            .query_rows(format!(
                "MATCH (:File {{id: '{file_path}'}})-[r:FILE_PART_OF]->(:Project {{id: '{}'}}) \
                 RETURN r.valid_at AS va, r.created_at AS ca, r.origin AS origin, \
                        r.invalid_at AS ia, r.expired_at AS ea",
                p.id
            ))
            .await
            .unwrap();
        // `as_i64` answers 0 for a NULL cell, so "stamped" and "not stamped" are
        // told apart by the value being a real clock reading rather than a
        // default - which is the whole assertion.
        let valid_at = row.rows[0][0].as_i64();
        let created_at = row.rows[0][1].as_i64();
        assert!(valid_at >= before, "the interval opens at write time, not at zero");
        assert_eq!(valid_at, created_at, "observed and recorded at the same instant");
        assert_eq!(
            row.rows[0][2].as_str(),
            "graph",
            "the system observed this membership; nobody asserted it"
        );
        // The OPEN end of the interval, and the reason every liveness read still
        // works unchanged: an edge that has not been closed carries no close.
        assert!(matches!(row.rows[0][3], CellValue::Null), "invalid_at stays open");
        assert!(matches!(row.rows[0][4], CellValue::Null), "expired_at stays open");
    }

    #[tokio::test]
    async fn link_file_stamps_the_hlc_when_a_merge_clock_is_attached() {
        // GD-R5: with a merge clock attached, a promoted membership carries the
        // HLC + device id so a future cross-device merge can order it. Without a
        // clock (the default) the columns stay NULL.
        let (store, _tmp) = setup().await;
        let store = store.with_clock(Arc::new(DeviceClock::new("dev-test".into())));
        let p = Project::new_inferred("test".into(), "/a".into(), 90);
        store.create(&p).await.unwrap();
        let path = "/a/main.rs";
        store
            .graph
            .write(format!(
                "CREATE (f:File {{id: '{path}', path: '{path}', app_id: 'test', last_accessed: 0}})"
            ))
            .await
            .unwrap();
        store.link_file(path, p.id).await.unwrap();
        let row = store
            .graph
            .query_rows(format!(
                "MATCH (:File {{id: '{path}'}})-[r:FILE_PART_OF]->(:Project {{id: '{}'}}) \
                 RETURN r.device_id, r.hlc_physical",
                p.id
            ))
            .await
            .unwrap();
        assert_eq!(row.rows[0][0].as_str(), "dev-test", "the device id is stamped");
        assert!(
            row.rows[0][1].as_i64() > 0,
            "the hlc physical stamp is present (a positive micros value), not NULL"
        );
    }

    #[tokio::test]
    async fn test_unlink_all_files() {
        let (store, _tmp) = setup().await;

        let p = Project::new_inferred("test".into(), "/a".into(), 90);
        store.create(&p).await.unwrap();

        // Create two file nodes.
        for path in ["/a/one.rs", "/a/two.rs"] {
            store
                .graph
                .write(format!(
                    "CREATE (f:File {{id: '{path}', path: '{path}', \
                     app_id: 'test', last_accessed: 0}})"
                ))
                .await
                .unwrap();
            store.link_file(path, p.id).await.unwrap();
        }

        assert_eq!(store.get_project_files(p.id).await.unwrap().len(), 2);

        store.unlink_all_files(p.id).await.unwrap();
        assert_eq!(store.get_project_files(p.id).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_explicit_project_is_promoted() {
        let (store, _tmp) = setup().await;

        let p = Project::new_explicit(Uuid::now_v7(), "explicit".into(), "/a".into());
        assert!(p.promoted);
        assert!(!p.inferred);
        assert_eq!(p.confidence, 100);

        store.create(&p).await.unwrap();
        let got = store.get_by_id(p.id).await.unwrap().unwrap();
        assert!(got.promoted);
        assert!(!got.inferred);
    }

    #[tokio::test]
    async fn prune_or_archive_alive_root_is_noop() {
        let (store, tmp) = setup().await;

        // Use the temp dir itself as a guaranteed-existing path.
        let live_path = tmp.path().to_string_lossy().to_string();
        let p = Project::new_inferred("alive".into(), live_path, 90);
        store.create(&p).await.unwrap();

        let outcome = store.prune_or_archive(p.id).await.unwrap();
        assert_eq!(outcome, PruneOutcome::Alive);

        // Project must still exist in active state.
        let got = store.get_by_id(p.id).await.unwrap().unwrap();
        assert_eq!(got.status, ProjectStatus::Active);
    }

    #[tokio::test]
    async fn prune_or_archive_dead_inferred_is_deleted() {
        let (store, _tmp) = setup().await;

        let p = Project::new_inferred(
            "dead-inferred".into(),
            "/no-such-path-1234567890/abcdef".into(),
            85,
        );
        store.create(&p).await.unwrap();

        let outcome = store.prune_or_archive(p.id).await.unwrap();
        assert_eq!(outcome, PruneOutcome::Pruned);

        // Closed rather than removed: the record has no delete.
        let got = store.get_by_id(p.id).await.unwrap().expect("kept as history");
        assert!(got.expired_at.is_some(), "inferred-dead project is closed");
        assert!(store.list_active().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn prune_or_archive_dead_explicit_is_archived() {
        let (store, _tmp) = setup().await;

        let p = Project::new_explicit(
            Uuid::now_v7(),
            "dead-explicit".into(),
            "/no-such-path-9876543210/zyxwvu".into(),
        );
        store.create(&p).await.unwrap();

        let outcome = store.prune_or_archive(p.id).await.unwrap();
        assert_eq!(outcome, PruneOutcome::Archived);

        // Node still present, but archived — explicit projects keep history.
        let got = store.get_by_id(p.id).await.unwrap().unwrap();
        assert_eq!(got.status, ProjectStatus::Archived);
    }

    #[tokio::test]
    async fn prune_or_archive_already_archived_is_idempotent() {
        let (store, _tmp) = setup().await;

        let p = Project::new_explicit(Uuid::now_v7(), "old".into(), "/gone".into());
        store.create(&p).await.unwrap();
        store.archive(p.id).await.unwrap();

        let outcome = store.prune_or_archive(p.id).await.unwrap();
        assert_eq!(outcome, PruneOutcome::AlreadyArchived);
    }

    #[tokio::test]
    async fn prune_dead_projects_aggregates_outcomes() {
        let (store, tmp) = setup().await;

        let live_path = tmp.path().to_string_lossy().to_string();
        let alive = Project::new_inferred("alive".into(), live_path, 90);
        let dead_inf = Project::new_inferred(
            "dead-inferred".into(),
            "/no-such-path-aaa/bbb".into(),
            70,
        );
        let dead_exp = Project::new_explicit(
            Uuid::now_v7(),
            "dead-explicit".into(),
            "/no-such-path-ccc/ddd".into(),
        );
        store.create(&alive).await.unwrap();
        store.create(&dead_inf).await.unwrap();
        store.create(&dead_exp).await.unwrap();

        let stats = store.prune_dead_projects().await.unwrap();
        assert_eq!(stats.alive, 1);
        assert_eq!(stats.pruned, 1);
        assert_eq!(stats.archived, 1);
        assert_eq!(stats.errors, 0);

        // Verify final graph state matches the stats.
        assert!(store.get_by_id(alive.id).await.unwrap().is_some());
        assert!(store.get_by_id(dead_inf.id).await.unwrap().unwrap().expired_at.is_some());
        let exp_after = store.get_by_id(dead_exp.id).await.unwrap().unwrap();
        assert_eq!(exp_after.status, ProjectStatus::Archived);
    }
}
