/// Integrity checks for SQLite and graph data.

use std::path::PathBuf;

use thiserror::Error;

/// Results of a full integrity check.
#[derive(Debug, Clone, Default)]
pub struct IntegrityReport {
    pub sqlite_ok: bool,
    pub graph_ok: bool,
    pub orphan_references: Vec<OrphanRef>,
    pub missing_schemas: Vec<String>,
    pub corrupt_entities: Vec<CorruptEntity>,
}

impl IntegrityReport {
    /// Whether the database is healthy (no issues found).
    pub fn is_healthy(&self) -> bool {
        self.sqlite_ok
            && self.graph_ok
            && self.orphan_references.is_empty()
            && self.corrupt_entities.is_empty()
    }

    /// Total number of issues found.
    pub fn issue_count(&self) -> usize {
        let mut count = 0;
        if !self.sqlite_ok {
            count += 1;
        }
        if !self.graph_ok {
            count += 1;
        }
        count += self.orphan_references.len();
        count += self.missing_schemas.len();
        count += self.corrupt_entities.len();
        count
    }
}

/// A reference to a non-existent entity.
#[derive(Debug, Clone)]
pub struct OrphanRef {
    pub source_id: String,
    pub source_type: String,
    pub field: String,
    pub target_id: String,
}

/// An entity that doesn't match its schema.
#[derive(Debug, Clone)]
pub struct CorruptEntity {
    pub id: String,
    pub entity_type: String,
    pub error: String,
}

/// Integrity check errors.
#[derive(Debug, Error)]
pub enum IntegrityError {
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("database: {0}")]
    Database(String),
}

/// Runs integrity checks on the SQLite event store and graph database.
pub struct IntegrityChecker {
    db_path: PathBuf,
    graph_path: PathBuf,
}

impl IntegrityChecker {
    pub fn new(db_path: PathBuf, graph_path: PathBuf) -> Self {
        Self { db_path, graph_path }
    }

    /// Run an integrity check. Both stores are verified for real: the SQLite
    /// event store via [`quick_check`](Self::quick_check) and the graph database
    /// via [`graph_check`](Self::graph_check). The deeper graph checks (orphan
    /// references, schema validation, corrupt entities) are NOT yet implemented,
    /// so those fields keep their empty defaults - `graph_ok` here means only
    /// "the graph opens and answers a query", not "every edge is consistent".
    pub async fn check(&self) -> IntegrityReport {
        IntegrityReport {
            sqlite_ok: self.quick_check().await,
            graph_ok: self.graph_check().await,
            orphan_references: vec![],
            missing_schemas: vec![],
            corrupt_entities: vec![],
        }
    }

    /// Graph check: open the graph database and run a trivial query. Returns
    /// `true` only when the store exists and the engine opens it and executes a
    /// query. Fail-closed: a missing graph directory, or one the engine cannot
    /// open (corrupt/locked), returns `false`. The `exists` guard is load-bearing
    /// - the engine would otherwise CREATE an empty graph for a missing path and
    /// wrongly report it sound, so a backup whose graph is absent is not sound.
    pub async fn graph_check(&self) -> bool {
        if !self.graph_path.exists() {
            return false;
        }
        let path = self.graph_path.to_string_lossy().into_owned();
        tokio::task::spawn_blocking(move || match crate::graph::spawn(&path) {
            Ok(handle) => handle.query_rows_sync("RETURN 1".to_string()).is_ok(),
            Err(_) => false,
        })
        .await
        .unwrap_or(false)
    }

    /// Quick check: run SQLite's `PRAGMA integrity_check` on the event store.
    /// Returns `true` only when the engine reports the database structurally
    /// sound (the first result row is `ok`). Fail-closed: a missing, unreadable,
    /// or non-SQLite file returns `false` - a backup that cannot even be opened
    /// and verified is not sound.
    pub async fn quick_check(&self) -> bool {
        self.sqlite_integrity_ok().await.unwrap_or(false)
    }

    /// Open the event store read-only and run `PRAGMA integrity_check`. `Ok(true)`
    /// iff the first result row is exactly `ok`; any open/query error propagates
    /// so the caller fails closed.
    async fn sqlite_integrity_ok(&self) -> Result<bool, IntegrityError> {
        let url = format!("sqlite:{}?mode=ro", self.db_path.display());
        let pool = sqlx::SqlitePool::connect(&url)
            .await
            .map_err(|e| IntegrityError::Database(e.to_string()))?;
        // `PRAGMA integrity_check` yields one `ok` row when sound, else a list of
        // problems. Reading the first row is enough to decide soundness.
        let row: (String,) = sqlx::query_as("PRAGMA integrity_check")
            .fetch_one(&pool)
            .await
            .map_err(|e| IntegrityError::Database(e.to_string()))?;
        pool.close().await;
        Ok(row.0 == "ok")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_healthy_report() {
        let r = IntegrityReport {
            sqlite_ok: true,
            graph_ok: true,
            ..Default::default()
        };
        assert!(r.is_healthy());
        assert_eq!(r.issue_count(), 0);
    }

    #[test]
    fn test_unhealthy_sqlite() {
        let r = IntegrityReport {
            sqlite_ok: false,
            graph_ok: true,
            ..Default::default()
        };
        assert!(!r.is_healthy());
        assert_eq!(r.issue_count(), 1);
    }

    #[test]
    fn test_unhealthy_orphans() {
        let r = IntegrityReport {
            sqlite_ok: true,
            graph_ok: true,
            orphan_references: vec![OrphanRef {
                source_id: "s1".into(),
                source_type: "Note".into(),
                field: "person_ref".into(),
                target_id: "missing".into(),
            }],
            ..Default::default()
        };
        assert!(!r.is_healthy());
        assert_eq!(r.issue_count(), 1);
    }

    #[test]
    fn test_corrupt_entity() {
        let r = IntegrityReport {
            sqlite_ok: true,
            graph_ok: true,
            corrupt_entities: vec![CorruptEntity {
                id: "e1".into(),
                entity_type: "Note".into(),
                error: "missing required field: title".into(),
            }],
            ..Default::default()
        };
        assert!(!r.is_healthy());
        assert_eq!(r.issue_count(), 1);
    }

    #[test]
    fn test_issue_count_combined() {
        let r = IntegrityReport {
            sqlite_ok: false,
            graph_ok: false,
            orphan_references: vec![
                OrphanRef { source_id: "a".into(), source_type: "X".into(), field: "f".into(), target_id: "b".into() },
            ],
            missing_schemas: vec!["com.missing".into()],
            corrupt_entities: vec![
                CorruptEntity { id: "c".into(), entity_type: "Y".into(), error: "bad".into() },
            ],
        };
        assert_eq!(r.issue_count(), 5);
    }

    #[tokio::test]
    async fn quick_check_fails_closed_on_a_missing_file() {
        let checker = IntegrityChecker::new(
            PathBuf::from("/tmp/arlen-integrity-nonexistent.db"),
            PathBuf::from("/tmp/arlen-integrity-nonexistent-graph"),
        );
        assert!(!checker.quick_check().await, "a backup that cannot be opened is not sound");
    }

    #[tokio::test]
    async fn quick_check_fails_closed_on_a_non_sqlite_file() {
        let p = std::env::temp_dir().join(format!("arlen-integrity-garbage-{}", std::process::id()));
        std::fs::write(&p, b"this is not a sqlite database").unwrap();
        let checker = IntegrityChecker::new(p.clone(), PathBuf::from("/tmp/unused-graph"));
        assert!(!checker.quick_check().await, "a non-SQLite file fails the integrity check");
        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn quick_check_passes_a_real_sqlite_db_and_check_reflects_it() {
        let p = std::env::temp_dir().join(format!("arlen-integrity-ok-{}.db", std::process::id()));
        let url = format!("sqlite:{}?mode=rwc", p.display());
        let pool = sqlx::SqlitePool::connect(&url).await.unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO t (v) VALUES ('x')").execute(&pool).await.unwrap();
        pool.close().await;

        let checker = IntegrityChecker::new(p.clone(), PathBuf::from("/tmp/unused-graph"));
        assert!(checker.quick_check().await, "a sound SQLite db passes integrity_check");
        std::fs::remove_file(&p).ok();
    }

    #[tokio::test]
    async fn graph_check_passes_a_real_graph_and_fails_a_missing_one() {
        let dir = std::env::temp_dir().join(format!("arlen-integrity-g-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let gpath = dir.join("graph");
        // Create a real graph via the low-level synchronous engine handle, which
        // drops (and releases the store lock) at the end of this block - so the
        // reopen by graph_check below does not race a detached worker thread.
        {
            let db = lbug::Database::new(gpath.to_str().unwrap(), lbug::SystemConfig::default())
                .expect("create graph");
            let _conn = lbug::Connection::new(&db).expect("connect");
        }
        let checker = IntegrityChecker::new(PathBuf::from("/tmp/unused.db"), gpath.clone());
        // Poll: dropping the handle releases the engine lock asynchronously.
        let mut ok = false;
        for _ in 0..40 {
            if checker.graph_check().await {
                ok = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(ok, "a real, openable graph passes graph_check");

        // A missing graph directory fails closed (and is NOT created).
        let missing = IntegrityChecker::new(PathBuf::from("/tmp/unused.db"), dir.join("no-such-graph"));
        assert!(!missing.graph_check().await, "a missing graph is not sound");
        assert!(!dir.join("no-such-graph").exists(), "graph_check must not create the graph");

        std::fs::remove_dir_all(&dir).ok();
    }
}
