//! The transfer policy config: `~/.config/arlen/transfer/policy.toml` (profile-system-plan.md).
//!
//! The on-disk shape of the directional rule table. `deny_unknown_fields`
//! (on [`crate::policy::TransferPolicy`] / [`crate::policy::TransferRule`]) keeps
//! a malformed or hostile file from parsing in extra structure. The load is
//! fail-closed: a file that does not parse yields an EMPTY policy (which
//! [`crate::policy::decide`] treats as default-deny), never a permissive
//! default - a broken policy grants no transfer, the OA-R1 "malformed config
//! grants nothing" discipline.
//!
//! Each `[[rule]]` is one ordered directed rule; order in the file is the
//! first-match order. A "both directions" Settings choice is written as two
//! rules. A Locked profile never appears as an allow here - the Locked-off
//! invariant is enforced in `decide`, not by the config.

use std::path::{Path, PathBuf};

use crate::policy::TransferPolicy;

/// The file name of the policy under the transfer config dir.
pub const POLICY_FILE: &str = "policy.toml";

/// An error loading the transfer policy.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("read: {0}")]
    Read(#[from] std::io::Error),
    /// The TOML did not parse or carried an unknown field.
    #[error("parse: {0}")]
    Parse(#[from] toml::de::Error),
}

/// The transfer config directory: `$XDG_CONFIG_HOME/arlen/transfer`, else
/// `$HOME/.config/arlen/transfer`. `None` when neither is set.
pub fn transfer_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("arlen").join("transfer"))
}

/// Parse a policy from TOML text. A parse failure is an error the caller turns
/// into the fail-closed empty policy (see [`load_policy`]).
pub fn parse_policy(contents: &str) -> Result<TransferPolicy, ConfigError> {
    Ok(toml::from_str(contents)?)
}

/// Load the policy from `dir/policy.toml`, fail-closed.
///
/// A missing file is an empty (default-deny) policy with no error - sealed by
/// default is the correct unconfigured state. A file that fails to read or parse
/// yields an empty policy AND the error, so the daemon can log the fault while
/// still refusing every transfer; a broken policy never grants access.
pub fn load_policy(dir: &Path) -> (TransferPolicy, Option<ConfigError>) {
    let path = dir.join(POLICY_FILE);
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Unconfigured: fully sealed, no error.
            return (TransferPolicy::default(), None);
        }
        Err(e) => return (TransferPolicy::default(), Some(ConfigError::Read(e))),
    };
    match parse_policy(&contents) {
        Ok(policy) => (policy, None),
        // A malformed policy grants nothing: empty + the error to log.
        Err(e) => (TransferPolicy::default(), Some(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Verdict;
    use crate::request::{ProfileId, TransferType};

    #[test]
    fn parses_directional_rules_in_order() {
        let toml = r#"
            [[rule]]
            source = "work"
            dest = "personal"
            ty = "file"
            allow = true

            [[rule]]
            source = "personal"
            dest = "work"
            ty = "clipboard"
            allow = false
        "#;
        let policy = parse_policy(toml).unwrap();
        assert_eq!(policy.rules.len(), 2);
        assert_eq!(policy.rules[0].source.as_str(), "work");
        assert!(policy.rules[0].allow);
        assert_eq!(policy.rules[0].ty, TransferType::File);
        assert!(!policy.rules[1].allow);
    }

    #[test]
    fn an_unknown_field_is_rejected_structurally() {
        let toml = r#"
            [[rule]]
            source = "work"
            dest = "personal"
            ty = "file"
            allow = true
            sneaky = "extra"
        "#;
        assert!(parse_policy(toml).is_err(), "deny_unknown_fields rejects extra structure");
    }

    #[test]
    fn a_malformed_policy_loads_to_default_deny() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(POLICY_FILE), "this is = not [valid").unwrap();
        let (policy, err) = load_policy(tmp.path());
        assert!(err.is_some(), "the parse error is surfaced, not swallowed");
        // The fail-closed empty policy denies everything.
        let work = ProfileId::new("work").unwrap();
        let personal = ProfileId::new("personal").unwrap();
        use crate::policy::{decide, ProfileRef};
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&work),
                &ProfileRef::unlocked(&personal),
                TransferType::File,
            ),
            Verdict::Deny,
            "a broken policy grants no transfer",
        );
    }

    #[test]
    fn a_missing_file_is_sealed_with_no_error() {
        let tmp = tempfile::tempdir().unwrap();
        let (policy, err) = load_policy(tmp.path());
        assert!(err.is_none(), "an unconfigured policy is not an error");
        assert!(policy.rules.is_empty(), "and is fully sealed (empty rules)");
    }
}
