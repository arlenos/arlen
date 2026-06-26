//! The per-app capability gate - the Arlen differentiator (online-accounts-plan.md).
//!
//! GOA/KDE let any app read the shared accounts DB and keyring (ambient access).
//! Arlen mediates EVERY token handout against a per-app capability grant, keyed on
//! the caller's existing identity (the F3 `path_to_app_id` model, resolved by the
//! daemon from the bus-attested PID at the D-Bus boundary). This is the pure
//! decision over the
//! loaded configs: given a resolved `caller_app_id`, `ListAccounts` returns only
//! the granted accounts and `GetAccessToken` is refused unless the app holds the
//! grant for that exact account + service. Fail-closed throughout: no grant, an
//! unknown account, or a service the grant omits all yield `Refused`.

use crate::config::{AccountConfig, Service};

/// The decision for one `GetAccessToken(account, service)` by a resolved caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    /// The app holds the grant; hand out a token at this least-privilege scope
    /// (`None` = the provider default for the service).
    Granted {
        /// The OAuth scope the grant maps to.
        scope: Option<String>,
    },
    /// No grant for this app on this account + service: hand out nothing.
    Refused,
}

/// Mediates account access for resolved callers against the loaded configs.
pub struct AccessGate<'a> {
    accounts: &'a [AccountConfig],
}

impl<'a> AccessGate<'a> {
    /// A gate over the daemon's loaded account set.
    pub fn new(accounts: &'a [AccountConfig]) -> Self {
        Self { accounts }
    }

    /// The accounts the caller's app holds ANY grant on - the only accounts
    /// `ListAccounts` may reveal to it. An app with no grant sees nothing (no
    /// shared-DB enumeration).
    pub fn granted_accounts(&self, caller_app_id: &str) -> Vec<&'a AccountConfig> {
        self.accounts
            .iter()
            .filter(|a| a.grants.iter().any(|g| g.app_id == caller_app_id))
            .collect()
    }

    /// The access decision for `caller_app_id` on `(account_id, service)`.
    /// `Granted` only when an account with that id **offers** this service AND
    /// carries a grant for this app that includes it; everything else is
    /// `Refused`. Requiring `account.services` too means an over-broad grant
    /// naming a service the account does not offer cannot mint a token (a grant
    /// can only ever narrow, never widen, what the account exposes).
    pub fn access(&self, caller_app_id: &str, account_id: &str, service: Service) -> Access {
        let Some(account) = self.accounts.iter().find(|a| a.id == account_id) else {
            return Access::Refused;
        };
        if !account.services.contains(&service) {
            return Access::Refused;
        }
        for grant in &account.grants {
            if grant.app_id == caller_app_id && grant.services.contains(&service) {
                return Access::Granted {
                    scope: grant.scope.clone(),
                };
            }
        }
        Access::Refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Grant, Service};

    fn account(id: &str, grants: Vec<Grant>) -> AccountConfig {
        AccountConfig {
            id: id.to_string(),
            provider: "google".into(),
            identity: "me@g.com".into(),
            presentation: None,
            services: vec![Service::Files, Service::Calendar],
            grants,
            files: None,
        }
    }

    fn grant(app: &str, services: Vec<Service>, scope: Option<&str>) -> Grant {
        Grant {
            app_id: app.to_string(),
            services,
            scope: scope.map(str::to_string),
        }
    }

    #[test]
    fn list_reveals_only_the_callers_granted_accounts() {
        let accounts = vec![
            account("a", vec![grant("org.arlen.files", vec![Service::Files], None)]),
            account("b", vec![grant("other.app", vec![Service::Files], None)]),
            account("c", vec![grant("org.arlen.files", vec![Service::Calendar], None)]),
        ];
        let gate = AccessGate::new(&accounts);
        let ids: Vec<&str> = gate
            .granted_accounts("org.arlen.files")
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "c"], "only accounts this app was granted");
        // An app with no grant sees nothing.
        assert!(gate.granted_accounts("ungranted.app").is_empty());
    }

    #[test]
    fn token_is_granted_only_for_the_exact_account_and_service() {
        let accounts = vec![account(
            "a",
            vec![grant(
                "org.arlen.files",
                vec![Service::Files],
                Some("drive.file"),
            )],
        )];
        let gate = AccessGate::new(&accounts);
        assert_eq!(
            gate.access("org.arlen.files", "a", Service::Files),
            Access::Granted {
                scope: Some("drive.file".into())
            }
        );
        // A service the grant omits is refused, even on a granted account.
        assert_eq!(
            gate.access("org.arlen.files", "a", Service::Calendar),
            Access::Refused
        );
        // A different app is refused.
        assert_eq!(gate.access("other.app", "a", Service::Files), Access::Refused);
        // An unknown account is refused (no oracle, fail-closed).
        assert_eq!(
            gate.access("org.arlen.files", "nonexistent", Service::Files),
            Access::Refused
        );
    }

    #[test]
    fn a_grant_for_a_service_the_account_does_not_offer_is_refused() {
        // The account offers Files + Calendar (the helper); a grant naming Mail
        // (not offered) must not mint a token - a grant can only narrow, never
        // widen, what the account exposes.
        let accounts = vec![account(
            "a",
            vec![grant("org.arlen.mail", vec![Service::Mail], Some("mail.all"))],
        )];
        let gate = AccessGate::new(&accounts);
        assert_eq!(gate.access("org.arlen.mail", "a", Service::Mail), Access::Refused);
    }

    #[test]
    fn empty_app_id_never_matches_a_grant() {
        // A failed identity resolution must not become an ambient key. The daemon
        // refuses before calling the gate, but the gate is fail-closed too: an
        // empty caller id matches no grant.
        let accounts = vec![account(
            "a",
            vec![grant("org.arlen.files", vec![Service::Files], None)],
        )];
        let gate = AccessGate::new(&accounts);
        assert!(gate.granted_accounts("").is_empty());
        assert_eq!(gate.access("", "a", Service::Files), Access::Refused);
    }
}
