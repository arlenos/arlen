//! SQLite-backed append-only ledger store and the tamper verifier.
//!
//! The store offers exactly two mutating operations: open (which
//! creates the schema) and [`append`](Ledger::append). There is no
//! update and no delete path in the code at all — the append-only
//! property of foundation §8.4.7 is enforced by the absence of those
//! operations, not by a runtime check.

use std::path::{Path, PathBuf};

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow,
};
use sqlx::{Row, SqlitePool};
use zeroize::Zeroizing;

use super::entry::{
    compute_entry_hash, AuditEntry, AuditKind, ForensicRecord, StructuralRecord,
    StructuralView, GENESIS_PREV_HASH,
};
use crate::checkpoint::{self, Checkpoint};
use crate::error::{AuditError, Result};

/// Largest page the read API will return in one request. A larger
/// `limit` is clamped to this so one request cannot pull the whole
/// ledger into memory.
pub const MAX_READ_LIMIT: u64 = 1000;

/// The append-only audit ledger.
///
/// Holds the SQLite pool, the HMAC key, and the in-memory chain head
/// (`next_index` + `prev_hash`). The head is loaded from the last
/// on-disk entry at [`open`](Self::open), so a crash between an
/// `INSERT` and the in-memory update is recovered correctly on the
/// next start.
pub struct Ledger {
    pool: SqlitePool,
    key: Zeroizing<Vec<u8>>,
    next_index: u64,
    prev_hash: [u8; 32],
    /// Path of the head checkpoint written after every append. Lets
    /// startup detect truncation the hash chain cannot (see
    /// [`crate::checkpoint`]).
    checkpoint_path: PathBuf,
}

impl Ledger {
    /// Open (creating if absent) the ledger at `db_path`, signing
    /// entries with `key`. WAL mode lets the read API read while
    /// appends proceed.
    pub async fn open(db_path: &Path, key: Zeroizing<Vec<u8>>) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            // 0700 so only the owning user can reach the ledger.
            crate::ensure_private_dir(parent)?;
        }
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            // Keep query temp btrees (ORDER BY / GROUP BY sorts, transient
            // indices) in RAM, not a temp file under $TMPDIR/$SQLITE_TMPDIR.
            // The daemon self-confines with a Landlock write-fence to its data
            // + socket dirs (Tier-A #2), so a /tmp temp-file spill would EACCES;
            // every current query orders by the INTEGER PK rowid (no spill), so
            // this is belt-and-suspenders against a future non-PK ORDER BY.
            .pragma("temp_store", "MEMORY");
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
            checkpoint_path: checkpoint::checkpoint_path(db_path),
        })
    }

    /// The index the next [`append`](Self::append) will assign.
    pub fn next_index(&self) -> u64 {
        self.next_index
    }

    /// The ledger's current head as `(index, entry_hash_hex)`, or
    /// `None` for an empty ledger. Used by the startup checkpoint
    /// comparison.
    pub fn head_for_checkpoint(&self) -> Option<(u64, String)> {
        if self.next_index == 0 {
            None
        } else {
            Some((self.next_index - 1, hex(&self.prev_hash)))
        }
    }

    /// Path of the head checkpoint beside this ledger.
    pub fn checkpoint_path(&self) -> &Path {
        &self.checkpoint_path
    }

    /// The hex-encoded `entry_hash` of the entry at `index`, or `None`
    /// if no such row exists. Used by the startup checkpoint
    /// comparison to confirm the checkpointed entry is still present
    /// and unchanged.
    pub async fn entry_hash_hex_at(&self, index: u64) -> Result<Option<String>> {
        let row = sqlx::query("SELECT entry_hash FROM audit_entries WHERE idx = ?")
            .bind(index as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(map_sqlx)?;
        match row {
            None => Ok(None),
            Some(r) => {
                let hash: Vec<u8> = r.try_get("entry_hash").map_err(map_sqlx)?;
                Ok(Some(hex(&hash)))
            }
        }
    }

    /// Probe whether the ledger at `db_path` already holds entries,
    /// without needing the HMAC key. The daemon calls this at startup
    /// to tell genesis (an absent or empty ledger, where generating a
    /// fresh key is fine) apart from a fault (a populated ledger whose
    /// key file has gone missing).
    ///
    /// Emptiness is determined **positively** — the `audit_entries`
    /// table is genuinely absent from `sqlite_master`. Every other
    /// failure (a locked, corrupt, or unreadable database) propagates
    /// as an error, so a probe failure can never be mistaken for an
    /// empty ledger and trigger a re-key.
    pub async fn probe_has_entries(db_path: &Path) -> Result<bool> {
        if !db_path.exists() {
            return Ok(false);
        }
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(false)
            .read_only(true)
            // Query temp btrees in RAM, not /tmp (the Landlock write-fence
            // grants only the data + socket dirs); read queries can still sort.
            .pragma("temp_store", "MEMORY");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(map_sqlx)?;

        // Is the table there at all? A readable database with no
        // `audit_entries` table is a genuine empty/genesis ledger.
        // A failure of *this* query means the database itself is not
        // readable — that must propagate, not read as "empty".
        let table: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name = 'audit_entries'",
        )
        .fetch_optional(&pool)
        .await
        .map_err(map_sqlx)?;

        let has = match table {
            None => false,
            Some(_) => {
                let count: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM audit_entries")
                        .fetch_one(&pool)
                        .await
                        .map_err(map_sqlx)?;
                count > 0
            }
        };
        pool.close().await;
        Ok(has)
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

        // Record the new head durably outside the database, so a later
        // truncation (deleting rows or the whole file) is evident at
        // startup. A checkpoint write failure is fail-closed, NOT
        // swallowed: if the head cannot be witnessed, the append
        // returns an error so the caller (via the ingest socket) gets
        // `Unavailable` and does not proceed with the action. That
        // keeps the invariant "the action proceeded ⇒ the checkpoint
        // advanced" — a degraded checkpoint cannot leave a window of
        // acknowledged-but-unwitnessed entries that a later truncation
        // could erase silently. The row stays committed (harmless: its
        // caller failed closed and abandoned the action), and the next
        // successful append rewrites the checkpoint to the live head.
        let cp = Checkpoint {
            index,
            entry_hash_hex: hex(&entry_hash),
            // No TPM anchor wired at this seal site yet (follow-up threads it in).
            counter: 0,
        };
        if let Err(e) = checkpoint::write(&self.checkpoint_path, &cp) {
            tracing::error!(
                "audit checkpoint write failed (entry {index} committed but \
                 unwitnessed); failing the append closed: {e}"
            );
            return Err(e);
        }

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

/// Read-only view onto the ledger, for the read API.
///
/// It opens its own read-only SQLite pool, so range queries run
/// concurrently with appends (WAL mode) and never contend on the
/// append writer's lock. It exposes the **Structural tier only** —
/// the `forensic` column is not even selected, so Forensic-tier
/// content cannot leak through this path.
pub struct LedgerReader {
    pool: SqlitePool,
}

impl LedgerReader {
    /// Open a read-only handle on the ledger at `db_path`.
    pub async fn open(db_path: &Path) -> Result<Self> {
        let opts = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(false)
            .read_only(true)
            // Query temp btrees in RAM, not /tmp (the Landlock write-fence
            // grants only the data + socket dirs); read queries can still sort.
            .pragma("temp_store", "MEMORY");
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(opts)
            .await
            .map_err(map_sqlx)?;
        Ok(Self { pool })
    }

    /// One past the highest index among entries matching the same
    /// filter as the page, so a client can seek to the tail for the
    /// most recent entries. Returns 0 when nothing matches.
    ///
    /// The head is scoped to `project_id` (when set) for the same
    /// reason the page is: a project-scoped read must not disclose the
    /// global ledger volume. For an unfiltered read this is the global
    /// `MAX(idx) + 1` (the total entry count, since indices are
    /// contiguous from 0).
    pub async fn head(&self, project_id: Option<&str>) -> Result<u64> {
        let row = sqlx::query(
            "SELECT MAX(idx) AS max_idx FROM audit_entries
             WHERE (?1 IS NULL OR project_id = ?1)",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx)?;
        // `MAX(idx)` is NULL when no row matches; map that to head 0.
        let max_idx: Option<i64> = row.try_get("max_idx").map_err(map_sqlx)?;
        Ok(max_idx.map_or(0, |m| m as u64 + 1))
    }

    /// Read a page of entries as Structural-tier views.
    ///
    /// Returns entries with index in `[from, to)`, ascending, capped
    /// at `min(limit, MAX_READ_LIMIT)`. When `project_id` is `Some`,
    /// only entries recorded under that project are returned — the
    /// basis of the project-scoped export. The `forensic` column is
    /// never selected.
    pub async fn read_structural(
        &self,
        from: u64,
        to: u64,
        limit: u64,
        project_id: Option<&str>,
    ) -> Result<Vec<StructuralView>> {
        // `idx` is a SQLite INTEGER (i64); clamp the u64 bounds so a
        // value past i64::MAX (e.g. `to = u64::MAX` for "everything")
        // does not wrap to a negative bound and match nothing.
        let from = i64::try_from(from).unwrap_or(i64::MAX);
        let to = i64::try_from(to).unwrap_or(i64::MAX);
        let capped = limit.min(MAX_READ_LIMIT) as i64;

        let rows = sqlx::query(
            "SELECT idx, timestamp, kind, actor, structural,
                    call_chain_id, project_id, entry_hash
             FROM audit_entries
             WHERE idx >= ?1 AND idx < ?2
               AND (?3 IS NULL OR project_id = ?3)
             ORDER BY idx ASC LIMIT ?4",
        )
        .bind(from)
        .bind(to)
        .bind(project_id)
        .bind(capped)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx)?;

        rows.iter().map(decode_structural_view).collect()
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

/// Build a [`StructuralView`] from a read-API row. The query behind
/// this does not select the `forensic` column, so the Forensic tier
/// cannot reach a reader through this path.
fn decode_structural_view(row: &SqliteRow) -> Result<StructuralView> {
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
    let call_chain_id: Option<String> =
        row.try_get("call_chain_id").map_err(map_sqlx)?;
    let project_id: Option<String> =
        row.try_get("project_id").map_err(map_sqlx)?;
    let entry_hash: Vec<u8> = row.try_get("entry_hash").map_err(map_sqlx)?;
    Ok(StructuralView {
        index: idx as u64,
        timestamp_micros: timestamp,
        kind,
        actor,
        structural,
        call_chain_id,
        project_id,
        entry_hash_hex: hex(&entry_hash),
    })
}

/// Lowercase hex encoding of a byte slice.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
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

    fn key() -> Zeroizing<Vec<u8>> {
        Zeroizing::new(b"audit-test-key-0123456789".to_vec())
    }

    fn structural(outcome: &str) -> StructuralRecord {
        StructuralRecord {
            subject: "com.arlen.files".into(),
            node_types: vec!["File".into()],
            relations: vec![],
            result_count: Some(3),
            duration_ms: Some(8),
            outcome: outcome.into(),
            depth: None,
            capability_change: None,
        }
    }

    async fn open_temp(dir: &std::path::Path, key: Zeroizing<Vec<u8>>) -> Ledger {
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
    async fn head_for_checkpoint_and_entry_hash_track_the_last_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = open_temp(dir.path(), key()).await;
        // Empty ledger has no head.
        assert_eq!(ledger.head_for_checkpoint(), None);
        ledger
            .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
            .await
            .expect("append");
        ledger
            .append(AuditKind::ToolCall, "ai-daemon", &structural("ok"), None, None, None)
            .await
            .expect("append");
        // The head is the LAST index (next_index - 1 == 1 after two appends),
        // carrying that entry's hash. Pins the index arithmetic and that the
        // checkpoint head is a real value, not always-None.
        let (idx, head_hash) = ledger
            .head_for_checkpoint()
            .expect("a non-empty ledger has a head");
        assert_eq!(idx, 1);
        // entry_hash_hex_at returns the stored hash for a present row (matching
        // the head), and None for an absent index.
        let at_last = ledger.entry_hash_hex_at(1).await.expect("query");
        assert_eq!(at_last.as_deref(), Some(head_hash.as_str()));
        assert!(ledger.entry_hash_hex_at(0).await.expect("query").is_some());
        assert_eq!(ledger.entry_hash_hex_at(99).await.expect("query"), None);
    }

    #[test]
    fn now_micros_is_a_real_current_era_timestamp() {
        // A genuine wall-clock read, not a constant: after 2020-01-01 in micros.
        // Pins the helper against a stubbed 0 / 1 / -1.
        assert!(now_micros() > 1_577_836_800_000_000);
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
        let ledger = open_temp(dir.path(), Zeroizing::new(b"a-different-key".to_vec())).await;
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

    #[tokio::test]
    async fn deleting_the_last_row_is_caught_by_the_checkpoint() {
        // The hash chain alone cannot catch tail truncation: the
        // remaining prefix verifies fine. The head checkpoint, written
        // beside the database on every append, records that there were
        // more entries.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("ledger.db");
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            for _ in 0..3 {
                ledger
                    .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
                    .await
                    .unwrap();
            }
            // The checkpoint now records head index 2.
        }
        // Delete the newest row out of band — the chain prefix 0..=1
        // still links and re-hashes correctly.
        {
            let ledger = open_temp(dir.path(), key()).await;
            sqlx::query("DELETE FROM audit_entries WHERE idx = 2")
                .execute(&ledger.pool)
                .await
                .unwrap();
        }
        let ledger = open_temp(dir.path(), key()).await;
        // The chain verifier is satisfied by the truncated prefix...
        assert_eq!(ledger.verify().await.unwrap(), 2);
        // ...but the checkpoint witness flags the missing entry: the
        // checkpoint points at index 2, which no longer exists.
        let stored = checkpoint::read(ledger.checkpoint_path());
        let cp_index = stored
            .as_ref()
            .ok()
            .and_then(|o| o.as_ref())
            .map(|c| c.index)
            .unwrap();
        let entry_hash_at_cp = ledger.entry_hash_hex_at(cp_index).await.unwrap();
        assert_eq!(entry_hash_at_cp, None, "the checkpointed entry was deleted");
        assert!(
            matches!(
                checkpoint::assess_startup(
                    stored,
                    ledger.head_for_checkpoint().is_none(),
                    entry_hash_at_cp,
                ),
                checkpoint::StartupCheck::Tampered { .. }
            ),
            "tail truncation must be caught by the checkpoint"
        );
        assert!(db.exists());
    }

    #[tokio::test]
    async fn append_fails_closed_when_the_checkpoint_cannot_be_written() {
        // If the head cannot be witnessed, the append must fail rather
        // than acknowledge an unwitnessed entry: the caller then fails
        // closed and does not perform the action. This keeps "action
        // proceeded ⇒ checkpoint advanced", closing the stale-
        // checkpoint truncation hole.
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = open_temp(dir.path(), key()).await;
        // Database stays writable (tempdir); only the checkpoint target
        // is unwritable — its parent directory does not exist.
        ledger.checkpoint_path =
            PathBuf::from("/nonexistent-audit-dir/head.checkpoint");
        let result = ledger
            .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
            .await;
        assert!(
            result.is_err(),
            "an unwritable checkpoint must fail the append closed"
        );
    }

    #[tokio::test]
    async fn deleting_the_whole_database_is_caught_by_the_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("ledger.db");
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            ledger
                .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
                .await
                .unwrap();
        }
        // Delete the database (and its WAL sidecars), leaving the
        // checkpoint behind — the naive "wipe the audit log" attack.
        let _ = std::fs::remove_file(&db);
        let _ = std::fs::remove_file(dir.path().join("ledger.db-wal"));
        let _ = std::fs::remove_file(dir.path().join("ledger.db-shm"));

        // Reopening recreates an empty database — a silent fresh start
        // without the checkpoint.
        let ledger = open_temp(dir.path(), key()).await;
        assert_eq!(ledger.verify().await.unwrap(), 0);
        assert_eq!(
            ledger.head_for_checkpoint(),
            None,
            "reopened database is empty"
        );
        let stored = checkpoint::read(ledger.checkpoint_path());
        let cp_index = stored
            .as_ref()
            .ok()
            .and_then(|o| o.as_ref())
            .map(|c| c.index)
            .unwrap();
        let entry_hash_at_cp = ledger.entry_hash_hex_at(cp_index).await.unwrap();
        assert!(
            matches!(
                checkpoint::assess_startup(stored, true, entry_hash_at_cp),
                checkpoint::StartupCheck::Tampered { .. }
            ),
            "whole-database deletion must be caught by the checkpoint"
        );
    }

    #[tokio::test]
    async fn probe_reports_an_absent_ledger_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("ledger.db");
        assert!(!Ledger::probe_has_entries(&missing).await.unwrap());
    }

    #[tokio::test]
    async fn probe_reports_an_empty_ledger_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        // Opening creates the schema but writes no rows.
        let _ledger = open_temp(dir.path(), key()).await;
        drop(_ledger);
        let db = dir.path().join("ledger.db");
        assert!(!Ledger::probe_has_entries(&db).await.unwrap());
    }

    #[tokio::test]
    async fn probe_reports_a_populated_ledger_as_nonempty() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            ledger
                .append(AuditKind::Query, "ai-daemon", &structural("ok"), None, None, None)
                .await
                .unwrap();
        }
        let db = dir.path().join("ledger.db");
        assert!(Ledger::probe_has_entries(&db).await.unwrap());
    }

    #[tokio::test]
    async fn probe_propagates_an_error_on_a_corrupt_db() {
        // A non-SQLite file at the ledger path must surface as an
        // error, never be mistaken for an empty (genesis) ledger —
        // that mistake would re-key a populated chain.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("ledger.db");
        std::fs::write(&db, b"this is not a sqlite database").unwrap();
        assert!(
            Ledger::probe_has_entries(&db).await.is_err(),
            "a corrupt ledger must propagate an error, not read as empty"
        );
    }

    /// Append `n` entries, each tagged with the given project id.
    async fn append_n(ledger: &mut Ledger, n: usize, project: Option<&str>) {
        for _ in 0..n {
            ledger
                .append(
                    AuditKind::Query,
                    "ai-daemon",
                    &structural("ok"),
                    None,
                    None,
                    project,
                )
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn reader_returns_a_half_open_index_range() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            append_n(&mut ledger, 5, None).await;
        }
        let reader = LedgerReader::open(&dir.path().join("ledger.db"))
            .await
            .unwrap();
        let page = reader.read_structural(1, 4, 100, None).await.unwrap();
        let indices: Vec<u64> = page.iter().map(|e| e.index).collect();
        assert_eq!(indices, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn head_is_one_past_the_highest_index_unfiltered() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            append_n(&mut ledger, 4, None).await;
        }
        let reader = LedgerReader::open(&dir.path().join("ledger.db"))
            .await
            .unwrap();
        assert_eq!(reader.head(None).await.unwrap(), 4);
    }

    #[tokio::test]
    async fn head_is_zero_for_an_empty_ledger() {
        let dir = tempfile::tempdir().unwrap();
        {
            // Touch the ledger so the db file exists, append nothing.
            let _ = open_temp(dir.path(), key()).await;
        }
        let reader = LedgerReader::open(&dir.path().join("ledger.db"))
            .await
            .unwrap();
        assert_eq!(reader.head(None).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn head_is_scoped_to_the_project_filter() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            // Indices 0,1 belong to project "p"; 2,3,4 are unscoped.
            append_n(&mut ledger, 2, Some("p")).await;
            append_n(&mut ledger, 3, None).await;
        }
        let reader = LedgerReader::open(&dir.path().join("ledger.db"))
            .await
            .unwrap();
        // Unfiltered head sees the whole ledger.
        assert_eq!(reader.head(None).await.unwrap(), 5);
        // Project-scoped head is one past the highest matching index
        // (1), never disclosing the global volume.
        assert_eq!(reader.head(Some("p")).await.unwrap(), 2);
        // A project with no entries reports head 0, not the global count.
        assert_eq!(reader.head(Some("absent")).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn reader_respects_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            append_n(&mut ledger, 5, None).await;
        }
        let reader = LedgerReader::open(&dir.path().join("ledger.db"))
            .await
            .unwrap();
        let page = reader.read_structural(0, u64::MAX, 2, None).await.unwrap();
        assert_eq!(page.len(), 2, "the page must honour the limit");
    }

    #[tokio::test]
    async fn reader_to_u64_max_returns_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            append_n(&mut ledger, 3, None).await;
        }
        let reader = LedgerReader::open(&dir.path().join("ledger.db"))
            .await
            .unwrap();
        // `to = u64::MAX` must not wrap to a negative SQLite bound.
        let page = reader.read_structural(0, u64::MAX, 100, None).await.unwrap();
        assert_eq!(page.len(), 3);
    }

    #[tokio::test]
    async fn reader_filters_by_project() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut ledger = open_temp(dir.path(), key()).await;
            append_n(&mut ledger, 2, Some("proj-a")).await;
            append_n(&mut ledger, 3, Some("proj-b")).await;
            append_n(&mut ledger, 1, None).await;
        }
        let reader = LedgerReader::open(&dir.path().join("ledger.db"))
            .await
            .unwrap();
        let only_a = reader
            .read_structural(0, u64::MAX, 100, Some("proj-a"))
            .await
            .unwrap();
        assert_eq!(only_a.len(), 2);
        assert!(only_a
            .iter()
            .all(|e| e.project_id.as_deref() == Some("proj-a")));
        // No filter returns all six.
        let all = reader.read_structural(0, u64::MAX, 100, None).await.unwrap();
        assert_eq!(all.len(), 6);
    }
}
