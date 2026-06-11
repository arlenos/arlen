//! Arlen online-accounts: the capability-gated account service (`org.arlen.Accounts1`).
//!
//! The differentiator over GOA/KDE: every account-token handout is mediated
//! against a per-app capability grant keyed on the caller's existing Arlen
//! identity (the F3 `path_to_app_id` model, here over the attested bus PID), not
//! ambient shared-DB access.
//! This crate is the daemon; this slice (OA-R1) is the account [`config`] model
//! and the per-app capability [`gate`]. The D-Bus ObjectManager + per-service
//! interfaces, the caller-auth resolution at the method boundary, and the Secret
//! Service token handout build on these.

pub mod config;
pub mod dbus;
pub mod gate;
pub mod rc;
