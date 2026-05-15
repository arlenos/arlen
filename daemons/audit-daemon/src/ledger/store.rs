//! SQLite-backed append-only ledger store and the tamper verifier.
//!
//! The store offers exactly two mutating operations: open (which
//! creates the schema) and [`append`](Ledger::append). There is no
//! update and no delete path in the code at all — the append-only
//! property of foundation §8.4.7 is enforced by the absence of those
//! operations, not by a runtime check.

use std::path::Path;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow,
};
use sqlx::{Row, SqlitePool};

use super::entry::{
    compute_entry_hash, AuditEntry, AuditKind, ForensicRecord, StructuralRecord,
    GENESIS_PREV_HASH,
};
use crate::error::{AuditError, Result};

/// The append-only audit ledger.
///
/// Holds the SQLite pool, the HMAC key, and the in-memory chain head
/// (`next_index` + `prev_hash`). The head is loaded from the last
/// on-disk entry at [`open`](Self::open), so a crash between an
/// `INSERT` and the in-memory update is recovered correctly on the
/// next start.
pub struct Ledger {
    pool: SqlitePool,
    key: Vec<u8>,
    next_index: u64,
    prev_hash: [u8; 32],
}

impl Ledger {
    /// Open (creating if absent) the ledger at `db_path`, signing
    /// entries with `key`. WAL mode lets the read API read while
    /// appends proceed.
    pub async fn open(db_path: &Path, key: Vec<u8>) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await
            .map_err(map_sqlx)?;
        create_schema(&pool).await?;
        let (next_index, prev_hash) = load_head(&pool).await?;
        Ok(Self {
            pool,
            key,
            next_index,
            prev_hash,
        })
    }

    /// The index the next [`append`](Self::append) will assign.
    pub fn next_index(&self) -> u64 {
        self.next_index
    }

    /// Append one entry. Returns the assigned chain index.
    ///
    /// The hash is computed, then the row is `INSERT`ed in a single
    /// statement (atomic), then the in-memory head advances — so a
    /// crash after the `INSERT` still leaves a consistent ledger that
    /// the next `open` picks up. A full device surfaces as
    /// [`AuditError::LedgerFull`] so the caller can fail closed.
    #[allow(clippy::too_many_arguments)]
    pub async fn append(
        &mut self,
        kind: AuditKind,
        actor: &str,
        structural: &StructuralRecord,
        forensic: Option<&ForensicRecord>,
        call_chain_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<u64> {
        let index = self.next_index;
        let timestamp = now_micros();
        let entry_hash = compute_entry_hash(
            &self.key,
            index,
            timestamp,
            kind,
            actor,
            structural,
            forensic,
            call_chain_id,
            project_id,
            &self.prev_hash,
        );

        let structural_json = serde_json::to_string(structural)
            .map_err(|e| AuditError::Storage(format!("encode structural: {e}")))?;
        let forensic_json = match forensic {
            Some(f) => Some(
                serde_json::to_string(f)
                    .map_err(|e| AuditError::Storage(format!("encode forensic: {e}")))?,
            ),
            None => None,
        };

        sqlx::query(
            "INSERT INTO audit_entries
             (idx, timestamp, kind, actor, structural, forensic,
              call_chain_id, project_id, prev_hash, entry_hash)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(index as i64)
        .bind(timestamp)
        .bind(kind.as_str())
        .bind(actor)
        .bind(structural_json)
        .bind(forensic_json)
        .bind(call_chain_id)
        .bind(project_id)
        .bind(self.prev_hash.as_slice())
        .bind(entry_hash.as_slice())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;

        self.next_index += 1;
        self.prev_hash = entry_hash;
        Ok(index)
    }

    /// Walk the whole chain and verify its integrity. Returns the
    /// number of entries verified, or [`AuditError::ChainBroken`] at
    /// the first entry whose index, linkage, or HMAC does not hold —
    /// catching any out-of-band edit, deletion, or insertion.
    pub async fn verify(&self) -> Result<u64> {
        let rows = sqlx::query(
            "SELECT idx, timestamp, kind, actor, structural, forensic,
                    call_chain_id, project_id, prev_hash, entry_hash
             FROM audit_entries ORDER BY idx ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        let mut expected_index: u64 = 0;
        let mut expected_prev = GENESIS_PREV_HASH;
        for row in &rows {
            let entry = decode_row(row)?;
            if entry.index != expected_index {
                return Err(AuditError::ChainBroken {
                    index: entry.index,
                    detail: format!(
                        "expected index {expected_index}, found {}",
                        entry.index
                    ),
                });
            }
            if entry.prev_hash != expected_prev {
                return Err(AuditError::ChainBroken {
                    index: entry.index,
                    detail: "prev_hash does not link to the previous entry".into(),
                });
            }
            let recomputed = compute_entry_hash(
                &self.key,
                entry.index,
                entry.timestamp_micros,
                entry.kind,
                &entry.actor,
                &entry.structural,
                entry.forensic.as_ref(),
                entry.call_chain_id.as_deref(),
                entry.project_id.as_deref(),
                &entry.prev_hash,
            );
            if recomputed != entry.entry_hash {
                return Err(AuditError::ChainBroken {
                    index: entry.index,
                    detail: "entry_hash does not match the recomputed HMAC".into(),
                });
            }
            expected_prev = entry.entry_hash;
            expected_index += 1;
        }
        Ok(expected_index)
    }
}

/// Create the append-only schema. `idx` is the chain order.
async fn create_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS audit_entries (
            idx           INTEGER PRIMARY KEY,
            timestamp     INTEGER NOT NULL,
            kind          TEXT    NOT NULL,
            actor         TEXT    NOT NULL,
            structural    TEXT    NOT NULL,
            forensic      TEXT,
            call_chain_id TEXT,
            project_id    TEXT,
            prev_hash     BLOB    NOT NULL,
            entry_hash    BLOB    NOT NULL
        )",
    )
    .execute(pool)
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

/// Load the chain head from the last on-disk entry. An empty ledger
/// starts at index 0 chaining from [`GENESIS_PREV_HASH`].
async fn load_head(pool: &SqlitePool) -> Result<(u64, [u8; 32])> {
    let row = sqlx::query(
        "SELECT idx, entry_hash FROM audit_entries ORDER BY idx DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(map_sqlx)?;
    match row {
        None => Ok((0, GENESIS_PREV_HASH)),
        Some(r) => {
            let idx: i64 = r.try_get("idx").map_err(map_sqlx)?;
            let hash: Vec<u8> = r.try_get("entry_hash").map_err(map_sqlx)?;
            Ok((idx as u64 + 1, to_hash(&hash)?))
        }
    }
}

/// Reconstruct an [`AuditEntry`] from one ledger row.
fn decode_row(row: &SqliteRow) -> Result<AuditEntry> {
    let idx: i64 = row.try_get("idx").map_err(map_sqlx)?;
    let timestamp: i64 = row.try_get("timestamp").map_err(map_sqlx)?;
    let kind_str: String = row.try_get("kind").map_err(map_sqlx)?;
    let kind = AuditKind::from_wire(&kind_str).ok_or_else(|| {
        AuditError::Storage(format!("unknown audit kind '{kind_str}'"))
    })?;
    let actor: String = row.try_get("actor").map_err(map_sqlx)?;
    let structural_json: String = row.try_get("structural").map_err(map_sqlx)?;
    let structural: StructuralRecord = serde_json::from_str(&structural_json)
        .map_err(|e| AuditError::Storage(format!("decode structural: {e}")))?;
    let forensic_json: Option<String> =
        row.try_get("forensic").map_err(map_sqlx)?;
    let forensic = match forensic_json {
        Some(j) => Some(
            serde_json::from_str(&j)
                .map_err(|e| AuditError::Storage(format!("decode forensic: {e}")))?,
        ),
        None => None,
    };
    let call_chain_id: Option<String> =
        row.try_get("call_chain_id").map_err(map_sqlx)?;
    let project_id: Option<String> =
        row.try_get("project_id").map_err(map_sqlx)?;
    let prev_hash: Vec<u8> = row.try_get("prev_hash").map_err(map_sqlx)?;
    let entry_hash: Vec<u8> = row.try_get("entry_hash").map_err(map_sqlx)?;
    Ok(AuditEntry {
        index: idx as u64,
        timestamp_micros: timestamp,
        kind,
        actor,
        structural,
        forensic,
        call_chain_id,
        project_id,
        prev_hash: to_hash(&prev_hash)?,
        entry_hash: to_hash(&entry_hash)?,
    })
}

/// Convert a hash column to a fixed 32-byte array. A wrong length is
/// a corrupt ledger, reported as a storage error.
fn to_hash(bytes: &[u8]) -> Result<[u8; 32]> {
    bytes.try_into().map_err(|_| {
        AuditError::Storage(format!(
            "corrupt hash column: expected 32 bytes, found {}",
            bytes.len()
        ))
    })
}

/// Current wall-clock time in microseconds since the Unix epoch.
fn now_micros() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

/// Map a SQLite error, distinguishing a full device (which the
/// ingest layer turns into `AuditUnavailable`) from other failures.
fn map_sqlx(e: sqlx::Error) -> AuditError {
    if let sqlx::Error::Database(db) = &e {
        // SQLITE_FULL has primary result code 13.
        if db.code().as_deref() == Some("13") {
            return AuditError::LedgerFull;
        }
    }
    AuditError::Storage(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> Vec<u8> {
        b"audit-test-key-0123456789".to_vec()
    }

    fn structural(outcome: &str) -> StructuralRecord {
        StructuralRecord {
            subject: "com.lunaris.files".into(),
            node_types: vec!["File".into()],
            relations: vec![],
            result_count: Some(3),
            duration_ms: Some(8),
            outcome: outcome.into(),
            depth: None,
        }
    }

    async fn open_temp(dir: &std::path::Path, key: Vec<u8>) -> Ledger {
        Ledger::open(&dir.join("ledger.db"), key)
            .await
            .expect("open ledger")
    }

    #[tokio::test]
    async fn empty_ledger_verifies_and_starts_at_zero() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = open_temp(dir.path(), key()).await;
        assert_eq!(ledger.next_index(), 0);
        assert_eq!(ledger.verify().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn append_assigns_sequential_indices_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = open_temp(dir.path(), key()).await;
        for i in 0..5 {
            let idx = ledger
                .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
                .await
                .expect("append");
            assert_eq!(idx, i);
        }
        assert_eq!(ledger.verify().await.unwrap(), 5);
    }

    #[tokio::test]
    async fn reopen_continues_the_chain() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            ledger
                .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
                .await
                .unwrap();
            ledger
                .append(AuditKind::ToolCall, "ai-daemon", &structural("ok"), None, None, None)
                .await
                .unwrap();
        }
        // A fresh handle must pick up the head from disk and keep the
        // chain unbroken across the reopen.
        let mut ledger = open_temp(dir.path(), key()).await;
        assert_eq!(ledger.next_index(), 2);
        let idx = ledger
            .append(AuditKind::Confirm, "ai-daemon", &structural("ok"), None, None, None)
            .await
            .unwrap();
        assert_eq!(idx, 2);
        assert_eq!(ledger.verify().await.unwrap(), 3);
    }

    #[tokio::test]
    async fn tampering_with_a_row_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = open_temp(dir.path(), key()).await;
        for _ in 0..3 {
            ledger
                .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
                .await
                .unwrap();
        }
        assert_eq!(ledger.verify().await.unwrap(), 3);

        // Mutate a row out of band — the kind of edit an attacker
        // with database access would make.
        sqlx::query("UPDATE audit_entries SET actor = 'evil' WHERE idx = 1")
            .execute(&ledger.pool)
            .await
            .unwrap();

        match ledger.verify().await {
            Err(AuditError::ChainBroken { index, .. }) => assert_eq!(index, 1),
            other => panic!("tampering not detected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn deleting_a_row_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = open_temp(dir.path(), key()).await;
        for _ in 0..3 {
            ledger
                .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
                .await
                .unwrap();
        }
        // Removing the middle entry breaks both the index sequence
        // and the prev_hash linkage of the entry after it.
        sqlx::query("DELETE FROM audit_entries WHERE idx = 1")
            .execute(&ledger.pool)
            .await
            .unwrap();
        assert!(matches!(
            ledger.verify().await,
            Err(AuditError::ChainBroken { .. })
        ));
    }

    #[tokio::test]
    async fn a_wrong_key_fails_verification() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            ledger
                .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
                .await
                .unwrap();
        }
        // Reopening with a different HMAC key cannot recompute the
        // stored hashes: the chain reads as broken from index 0.
        let ledger = open_temp(dir.path(), b"a-different-key".to_vec()).await;
        match ledger.verify().await {
            Err(AuditError::ChainBroken { index, .. }) => assert_eq!(index, 0),
            other => panic!("wrong key was not rejected: {other:?}"),
        }
    }

    #[tokio::test]
    async fn forensic_payload_round_trips_and_chains() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = open_temp(dir.path(), key()).await;
        let forensic = ForensicRecord {
            query_string: "files I edited yesterday".into(),
            parameters: "{}".into(),
            stack_trace: "ai-daemon::query".into(),
        };
        ledger
            .append(
                AuditKind::Query,
                "ai-daemon",
                &structural("ok"),
                Some(&forensic),
                Some("chain-1"),
                Some("proj-x"),
            )
            .await
            .unwrap();
        // The forensic payload is covered by the chain hash, so the
        // entry verifies only if it round-tripped intact.
        assert_eq!(ledger.verify().await.unwrap(), 1);
    }
}
