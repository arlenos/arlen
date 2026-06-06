//! Audit daemon error type.

use thiserror::Error;

/// Errors raised by the audit ledger and daemon.
#[derive(Debug, Error)]
pub enum AuditError {
    /// A storage-layer failure (open, query, or I/O against SQLite).
    #[error("ledger storage: {0}")]
    Storage(String),

    /// The ledger device is out of space. Distinct from
    /// [`Storage`](Self::Storage) because the ingest layer maps this
    /// to `AuditUnavailable`: per foundation §8.4.6 the AI daemon
    /// must refuse to act when its activity cannot be audited, never
    /// drop the entry silently.
    #[error("audit ledger is full")]
    LedgerFull,

    /// The HMAC key cannot be used in a way that forbids proceeding —
    /// most importantly, the key file is missing while the ledger
    /// already holds entries. Generating a fresh key would make the
    /// whole existing chain read as tampered, so the daemon fails
    /// closed instead of re-keying.
    #[error("audit key unavailable: {0}")]
    KeyUnavailable(String),

    /// The hash chain failed verification: an entry was modified,
    /// removed, or inserted out of band. The `index` is the first
    /// entry at which the chain does not hold.
    #[error("audit chain broken at index {index}: {detail}")]
    ChainBroken {
        /// Index of the first entry that fails verification.
        index: u64,
        /// What specifically did not hold (linkage, hash, ordering).
        detail: String,
    },

    /// An underlying I/O error.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl From<audit_proto::ProtoError> for AuditError {
    /// A wire-protocol failure on a socket is, to the audit daemon,
    /// a storage-layer failure of that exchange.
    fn from(e: audit_proto::ProtoError) -> Self {
        AuditError::Storage(e.to_string())
    }
}

/// Result alias for the audit daemon.
pub type Result<T> = std::result::Result<T, AuditError>;
