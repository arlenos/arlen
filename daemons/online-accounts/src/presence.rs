//! Peer presence tracking for directed account-change notifications (online-
//! accounts-plan.md §3.1 point 2). A vanilla D-Bus signal is BROADCAST to every
//! subscriber, which would re-open the ambient hole - an app learning that an
//! account it was never granted just changed. The fix is to UNICAST the
//! `AccountsChanged`-class signal only to the connections of the apps an account
//! is granted to.
//!
//! To unicast, the daemon must know which unique bus name belongs to which app.
//! It learns this as apps call it (each call carries the caller's sender bus name
//! and resolves to an F3 app-id), records the pair here, and prunes it when the
//! connection drops (`NameOwnerChanged` to the empty owner). [`recipients`] is the
//! no-leak core: it returns only the bus names whose app-id is in the account's
//! grant set, never any other connection.

use crate::config::AccountConfig;
use std::collections::HashMap;

/// What changed about an account between two config snapshots, for a directed
/// `AccountsChanged` signal.
#[derive(Debug, PartialEq, Eq)]
pub enum AccountChangeKind {
    /// The account is new in this snapshot.
    Added,
    /// The account is gone from this snapshot.
    Removed,
    /// The account exists in both snapshots but its config differs.
    Modified,
}

/// One account's change, keyed by account id.
#[derive(Debug, PartialEq, Eq)]
pub struct AccountChange {
    /// The account id that changed.
    pub account_id: String,
    /// How it changed.
    pub kind: AccountChangeKind,
}

/// Diff two account-config snapshots into the per-account changes to signal:
/// `Added` (new only), `Removed` (old only), `Modified` (in both but differing).
/// The account count is small, so a simple O(n^2) match by id is fine; the result
/// is sorted by account id for determinism. An unchanged account yields nothing.
pub fn diff_accounts(old: &[AccountConfig], new: &[AccountConfig]) -> Vec<AccountChange> {
    let mut changes = Vec::new();
    for n in new {
        match old.iter().find(|o| o.id == n.id) {
            None => changes.push(AccountChange {
                account_id: n.id.clone(),
                kind: AccountChangeKind::Added,
            }),
            Some(o) if o != n => changes.push(AccountChange {
                account_id: n.id.clone(),
                kind: AccountChangeKind::Modified,
            }),
            Some(_) => {}
        }
    }
    for o in old {
        if !new.iter().any(|n| n.id == o.id) {
            changes.push(AccountChange {
                account_id: o.id.clone(),
                kind: AccountChangeKind::Removed,
            });
        }
    }
    changes.sort_by(|a, b| a.account_id.cmp(&b.account_id));
    changes
}

/// Tracks unique-bus-name -> resolved-app-id for the currently-connected callers,
/// so an account-change signal reaches only granted apps' connections.
#[derive(Debug, Default)]
pub struct PeerRegistry {
    by_name: HashMap<String, String>,
}

impl PeerRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that the connection at `bus_name` resolved to `app_id` (called on
    /// each admitted request, so the mapping tracks live callers). Re-recording a
    /// name updates its app-id.
    pub fn record(&mut self, bus_name: impl Into<String>, app_id: impl Into<String>) {
        self.by_name.insert(bus_name.into(), app_id.into());
    }

    /// Forget a connection (its unique name vanished - `NameOwnerChanged` to no
    /// owner). A no-op if it was not tracked.
    pub fn forget(&mut self, bus_name: &str) {
        self.by_name.remove(bus_name);
    }

    /// The bus names to unicast an account change to: every tracked connection
    /// whose app-id is in `granted` (the account's grant set), and NEVER any
    /// other. An app with two connections gets both; an ungranted app gets none.
    /// Sorted for a deterministic result.
    pub fn recipients(&self, granted: &[String]) -> Vec<String> {
        let mut out: Vec<String> = self
            .by_name
            .iter()
            .filter(|(_, app)| granted.iter().any(|g| g == *app))
            .map(|(name, _)| name.clone())
            .collect();
        out.sort();
        out
    }

    /// How many connections are tracked.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether no connection is tracked.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// The unicast emit tuples for a batch of account changes: `(recipient_bus_name,
/// account_id, change_kind)`. For each change, only the connections of the apps
/// GRANTED that account are targets, resolved from the snapshot the account lives
/// in - the NEW one for `Added`/`Modified`, the OLD one for a `Removed` account
/// (whose grants are gone from the new snapshot). Never any other app: this is the
/// no-leak property (an app is not woken by an account it was not granted) carried
/// end-to-end. The change_kind is a stable lowercase label.
pub fn emit_targets<'a>(
    changes: &'a [AccountChange],
    old: &[AccountConfig],
    new: &[AccountConfig],
    peers: &PeerRegistry,
) -> Vec<(String, &'a str, &'static str)> {
    let mut out = Vec::new();
    for c in changes {
        let source = match c.kind {
            AccountChangeKind::Removed => old,
            _ => new,
        };
        let Some(account) = source.iter().find(|a| a.id == c.account_id) else {
            continue;
        };
        let granted: Vec<String> = account.grants.iter().map(|g| g.app_id.clone()).collect();
        let kind = match c.kind {
            AccountChangeKind::Added => "added",
            AccountChangeKind::Removed => "removed",
            AccountChangeKind::Modified => "modified",
        };
        for recipient in peers.recipients(&granted) {
            out.push((recipient, c.account_id.as_str(), kind));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn granted(apps: &[&str]) -> Vec<String> {
        apps.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_a_granted_apps_connection_is_a_recipient() {
        let mut r = PeerRegistry::new();
        r.record(":1.10", "com.example.mail");
        r.record(":1.11", "com.example.other");
        // The account is granted only to the mail app.
        assert_eq!(r.recipients(&granted(&["com.example.mail"])), vec![":1.10"]);
    }

    #[test]
    fn an_ungranted_app_is_never_woken() {
        let mut r = PeerRegistry::new();
        r.record(":1.20", "com.example.snoop");
        // No grant matches the snoop app: it must receive nothing.
        assert!(r.recipients(&granted(&["com.example.mail"])).is_empty());
        // Even with an empty grant set, nothing leaks.
        assert!(r.recipients(&[]).is_empty());
    }

    #[test]
    fn every_connection_of_a_granted_app_is_a_recipient() {
        let mut r = PeerRegistry::new();
        r.record(":1.30", "com.example.mail");
        r.record(":1.31", "com.example.mail"); // second window of the same app
        assert_eq!(
            r.recipients(&granted(&["com.example.mail"])),
            vec![":1.30", ":1.31"]
        );
    }

    #[test]
    fn a_forgotten_connection_stops_receiving() {
        let mut r = PeerRegistry::new();
        r.record(":1.40", "com.example.mail");
        r.forget(":1.40");
        assert!(r.is_empty());
        assert!(r.recipients(&granted(&["com.example.mail"])).is_empty());
    }

    use crate::config::{AccountConfig, Service};

    fn acct(id: &str, identity: &str) -> AccountConfig {
        AccountConfig {
            id: id.to_string(),
            provider: "nextcloud".to_string(),
            identity: identity.to_string(),
            presentation: None,
            services: vec![Service::Files],
            grants: vec![],
            files: None,
        }
    }

    #[test]
    fn diff_detects_added_removed_and_modified() {
        let old = vec![acct("keep", "a"), acct("gone", "b"), acct("edit", "c")];
        let new = vec![acct("keep", "a"), acct("edit", "c2"), acct("fresh", "d")];
        assert_eq!(
            diff_accounts(&old, &new),
            vec![
                AccountChange { account_id: "edit".into(), kind: AccountChangeKind::Modified },
                AccountChange { account_id: "fresh".into(), kind: AccountChangeKind::Added },
                AccountChange { account_id: "gone".into(), kind: AccountChangeKind::Removed },
            ]
        );
    }

    #[test]
    fn identical_snapshots_have_no_changes() {
        let s = vec![acct("a", "x")];
        assert!(diff_accounts(&s, &s).is_empty());
    }

    use crate::config::Grant;

    fn granted_acct(id: &str, app: &str) -> AccountConfig {
        let mut a = acct(id, "user@x");
        a.grants = vec![Grant {
            app_id: app.to_string(),
            services: vec![Service::Files],
            scope: None,
        }];
        a
    }

    #[test]
    fn emit_targets_reaches_only_the_granted_apps_connection() {
        let mut peers = PeerRegistry::new();
        peers.record(":1.10", "com.example.mail"); // granted
        peers.record(":1.11", "com.example.snoop"); // NOT granted
        let old = vec![granted_acct("work", "com.example.mail")];
        let mut edited = granted_acct("work", "com.example.mail");
        edited.identity = "changed@x".to_string();
        let new = vec![edited];
        let changes = diff_accounts(&old, &new);
        // Only the granted mail app's connection is targeted; snoop is never woken.
        assert_eq!(
            emit_targets(&changes, &old, &new, &peers),
            vec![(":1.10".to_string(), "work", "modified")]
        );
    }

    #[test]
    fn a_removed_account_notifies_its_old_grantee_from_the_old_snapshot() {
        let mut peers = PeerRegistry::new();
        peers.record(":1.20", "com.example.mail");
        let old = vec![granted_acct("gone", "com.example.mail")];
        let new: Vec<AccountConfig> = vec![];
        let changes = diff_accounts(&old, &new);
        assert_eq!(
            emit_targets(&changes, &old, &new, &peers),
            vec![(":1.20".to_string(), "gone", "removed")]
        );
    }
}
