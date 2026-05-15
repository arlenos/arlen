//! Lunaris Audit Daemon.
//!
//! `lunaris-auditd` is the sole writer of the system audit log
//! (foundation §8.4.7). Every Knowledge-Graph access, AI action, and
//! permission grant or denial is recorded as one entry in an
//! append-only, hash-chained ledger. Other components never write
//! the log directly: they send audit events over a restricted,
//! peer-authenticated IPC channel and this daemon appends them.
//!
//! Running the audit log as its own process is a deliberate security
//! boundary (§8.4.5): a component compromise — say a Graph Daemon
//! dumping the graph — cannot also suppress or forge the audit trail,
//! so the Anomaly Detector still sees the anomalous activity.
//!
//! Architecture: `docs/architecture/phase-9-gamma-plan.md`.
//!
//! S13.1 scope: the [`ledger`] core — the append-only store, the
//! HMAC hash-chain, and the tamper verifier. The ingest and read
//! sockets are layered on in S13.3 and S13.4.

#![forbid(unsafe_code)]

pub mod error;
pub mod ledger;

pub use error::{AuditError, Result};
