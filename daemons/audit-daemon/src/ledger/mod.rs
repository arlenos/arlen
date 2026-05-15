//! The append-only, hash-chained audit ledger.
//!
//! [`entry`] holds the entry types and the HMAC chain function;
//! [`store`] is the SQLite-backed append-only store and the tamper
//! verifier.

mod entry;
mod store;

pub use entry::{
    compute_entry_hash, AuditEntry, AuditKind, ForensicRecord, StructuralRecord,
    GENESIS_PREV_HASH,
};
pub use store::Ledger;
