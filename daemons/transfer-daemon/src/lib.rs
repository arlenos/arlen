//! Arlen transfer daemon: the sole cross-profile broker (`org.arlen.Transfer1`).
//!
//! By default profiles are fully sealed; this daemon is the only process with
//! simultaneous access to two profile namespaces, and all cross-profile flow
//! goes through it. It is Qubes-modeled (profile-system-plan.md, Decided 4-5):
//! a default-deny, first-match, DIRECTIONAL policy keyed on
//! (source-profile, dest-profile, type), every transfer audited to BOTH
//! profiles' ledgers, and transferred bytes treated as the highest-risk origin
//! on the receiving side (the confused-deputy close).
//!
//! This crate is the daemon; this slice (PR-R4 CORE) is the testable security
//! core:
//! - [`policy`] - the default-deny first-match directional decision, with the
//!   un-overridable Locked-off invariant.
//! - [`request`] - the transfer request model (intent + a handle, never bytes)
//!   and its fail-closed validation.
//! - [`audit`] - the dual-ledger wiring (both profiles' `arlen-auditd`,
//!   content-free, audit-before-act, both-must-succeed).
//! - [`auth`] - the requester allowlist (who may ASK for a transfer).
//! - [`gate`] - composes auth + validity + policy into the single decision
//!   surface, minting the broker's [`broker::ApprovedTransfer`] only on a
//!   recorded allow.
//! - [`receive`] - the receive-side confused-deputy seam (`ExternalContent`
//!   stamp + the S18-B sandbox routing decision).
//! - [`dbus`] - the request surface (a raw per-uid socket; the transport
//!   decision is documented there).
//! - [`config`] - the on-disk policy.
//!
//! DEFERRED (seam modeled, not built): the live dual-uid namespace-crossing
//! broker ([`broker::TransferBroker`], needs PR-R1 + two live profile uids), the
//! two-gesture clipboard handle lifecycle, the 3-second undo toast (UX, not the
//! security boundary), and the live `parse_document` call on delivery.

pub mod audit;
pub mod auth;
pub mod broker;
pub mod config;
pub mod dbus;
pub mod gate;
pub mod policy;
pub mod receive;
pub mod request;
