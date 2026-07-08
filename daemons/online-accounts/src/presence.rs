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

use std::collections::HashMap;

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
}
