//! Arlen online-accounts: the capability-gated account service (`org.arlen.Accounts1`).
//!
//! The differentiator over GOA/KDE: every account-token handout is mediated
//! against a per-app capability grant keyed on the caller's existing Arlen
//! identity (the F3 `path_to_app_id` model, here over the attested bus PID), not
//! ambient shared-DB access.
//! This crate is the daemon; the built surface is the account [`config`] model,
//! the per-app capability [`gate`], the caller-auth resolution at the method
//! boundary ([`dbus`]) and the Secret Service token [`vault`] handout. The
//! app-facing surface is the gated `ListAccounts`/`GetAccessToken` methods: each
//! resolves the caller and returns only its granted accounts.
//!
//! The D-Bus **ObjectManager** + per-account objects (`online-accounts-plan.md`
//! §3.1) are deliberately NOT built yet, and the reason is a security one, not
//! just a missing consumer: a standard `zbus::fdo::ObjectManager` plus ungated
//! per-account `Account` property objects would answer `GetManagedObjects` (and
//! direct property reads of `/Accounts/{id}`) for ANY caller. On the session bus
//! every app shares one uid, so that enumeration leaks every account's metadata
//! to every app - exactly the ambient-shared-DB hole over GOA/KDE this daemon
//! exists to close. The plan's "bus-policy-restricted to the management caller"
//! cannot distinguish same-uid callers, and the standard property-getter model
//! has no per-caller gate. A sound ObjectManager needs either F3-grade same-uid
//! caller distinction or a custom per-caller-scoped enumeration (the gate
//! `ListAccounts` already applies); building the naive surface would regress the
//! per-caller-visibility property. Until then, `ListAccounts` is the surface.

pub mod attenuate;
pub mod audit;
pub mod config;
pub mod connection;
pub mod dbus;
pub mod gate;
pub mod master;
pub mod mount;
pub mod objects;
pub mod presence;
pub mod watcher;
pub mod rc;
pub mod ssh_config;
pub mod vault;
