//! The account intent config: `~/.config/arlen/accounts/{id}.toml` (online-accounts-plan.md).
//!
//! Metadata/secret split: this file carries ONLY the account's intent (who it is,
//! which services it offers, which apps are granted which service). No token, no
//! password, no client secret ever lives here - those are in the Secret Service
//! under the per-app master-secret. `deny_unknown_fields` makes the no-secrets
//! rule structural: a config that tries to carry a `token`/`secret` field is
//! rejected, so a stray secret cannot be parsed in by accident.

use std::path::Path;

use serde::Deserialize;

/// A typed account service. A consumer requests only the interface it needs (the
/// file manager -> `Files`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Service {
    /// Cloud-drive files (the rclone-mounted drive).
    Files,
    /// Calendar (CalDAV).
    Calendar,
    /// Mail (IMAP).
    Mail,
    /// Contacts (CardDAV).
    Contacts,
    /// Photos.
    Photos,
}

impl Service {
    /// Parse the lowercase wire name a caller passes to `GetAccessToken`
    /// (`"files"`, `"calendar"`, …). An unknown service is `None` so the daemon
    /// refuses it rather than guessing a scope.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "files" => Service::Files,
            "calendar" => Service::Calendar,
            "mail" => Service::Mail,
            "contacts" => Service::Contacts,
            "photos" => Service::Photos,
            _ => return None,
        })
    }

    /// The lowercase wire name (the inverse of [`parse`]). Used as a coarse,
    /// content-free service label in the credential-handout audit.
    pub fn as_key(&self) -> &'static str {
        match self {
            Service::Files => "files",
            Service::Calendar => "calendar",
            Service::Mail => "mail",
            Service::Contacts => "contacts",
            Service::Photos => "photos",
        }
    }
}

/// One per-app capability grant: which app may use which of this account's
/// services, and the least-privilege OAuth scope the grant maps to. The presence
/// of a grant IS the capability - absence means no access (fail-closed).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    /// The Arlen app id the grant is for (matched against the caller's
    /// SO_PEERCRED + F3 `path_to_app_id` identity at access time).
    pub app_id: String,
    /// The services this app may use on this account.
    #[serde(default)]
    pub services: Vec<Service>,
    /// The least-privilege OAuth scope this grant maps to (`drive.file` /
    /// `drive.appfolder`), handed out with the token. `None` lets the daemon pick
    /// the provider default for the service.
    #[serde(default)]
    pub scope: Option<String>,
}

/// One account's intent. No secrets (see the module doc).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountConfig {
    /// Stable account id (also the config file stem).
    pub id: String,
    /// The provider key (`google`, `nextcloud`, ...).
    pub provider: String,
    /// The account identity (the login, e.g. an email).
    pub identity: String,
    /// A human presentation name for the UI, when set.
    #[serde(default)]
    pub presentation: Option<String>,
    /// The services this account offers.
    #[serde(default)]
    pub services: Vec<Service>,
    /// The per-app capability grants (`[[grant]]` blocks).
    #[serde(default, rename = "grant")]
    pub grants: Vec<Grant>,
}

/// An error loading one account config.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("read: {0}")]
    Read(#[from] std::io::Error),
    /// The TOML did not parse or carried an unknown (possibly secret) field.
    #[error("parse: {0}")]
    Parse(#[from] toml::de::Error),
    /// The in-file `id` does not match the file name stem.
    #[error("id {found:?} does not match file name {expected:?}")]
    IdMismatch {
        /// The `id` field inside the file.
        found: String,
        /// The file name stem the daemon resolved the account by.
        expected: String,
    },
}

/// The account config directory: `$XDG_CONFIG_HOME/arlen/accounts`, else
/// `$HOME/.config/arlen/accounts`. `None` when neither is set.
pub fn accounts_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("arlen").join("accounts"))
}

/// Parse one account config, requiring its `id` to match the file stem (so an
/// account is always reached by a consistent id and a misplaced file cannot
/// shadow another account).
pub fn parse_account(path: &Path, contents: &str) -> Result<AccountConfig, ConfigError> {
    let account: AccountConfig = toml::from_str(contents)?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    if account.id != stem {
        return Err(ConfigError::IdMismatch {
            found: account.id,
            expected: stem.to_string(),
        });
    }
    Ok(account)
}

/// Load every `{id}.toml` account config in `dir`. A file that fails to parse (or
/// whose id mismatches) is SKIPPED with its error returned alongside, never
/// silently granted: a malformed grant config yields no account, so it grants no
/// access (fail-closed). A missing directory is an empty set, not an error.
pub fn load_accounts(dir: &Path) -> (Vec<AccountConfig>, Vec<(std::path::PathBuf, ConfigError)>) {
    let mut accounts = Vec::new();
    let mut errors = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return (accounts, errors),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        match std::fs::read_to_string(&path).map_err(ConfigError::from) {
            Ok(contents) => match parse_account(&path, &contents) {
                Ok(account) => accounts.push(account),
                Err(e) => errors.push((path, e)),
            },
            Err(e) => errors.push((path, e)),
        }
    }
    (accounts, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn service_parse_round_trips_and_rejects_unknown() {
        assert_eq!(Service::parse("files"), Some(Service::Files));
        assert_eq!(Service::parse("calendar"), Some(Service::Calendar));
        assert_eq!(Service::parse("mail"), Some(Service::Mail));
        assert_eq!(Service::parse("contacts"), Some(Service::Contacts));
        assert_eq!(Service::parse("photos"), Some(Service::Photos));
        // Unknown or mis-cased names are refused (the daemon won't guess a scope).
        assert_eq!(Service::parse("Files"), None);
        assert_eq!(Service::parse("drive"), None);
        assert_eq!(Service::parse(""), None);
    }

    #[test]
    fn parses_an_account_with_grants() {
        let toml = r#"
            id = "gdrive-personal"
            provider = "google"
            identity = "me@gmail.com"
            presentation = "Personal Drive"
            services = ["files", "calendar"]

            [[grant]]
            app_id = "org.arlen.files"
            services = ["files"]
            scope = "drive.file"

            [[grant]]
            app_id = "settings"
            services = ["files", "calendar"]
        "#;
        let a = parse_account(Path::new("/x/gdrive-personal.toml"), toml).unwrap();
        assert_eq!(a.provider, "google");
        assert_eq!(a.grants.len(), 2);
        assert_eq!(a.grants[0].app_id, "org.arlen.files");
        assert_eq!(a.grants[0].services, vec![Service::Files]);
        assert_eq!(a.grants[0].scope.as_deref(), Some("drive.file"));
    }

    #[test]
    fn a_secret_field_is_rejected_structurally() {
        // The no-secrets rule is enforced by deny_unknown_fields: a token field
        // cannot be parsed in.
        let toml = r#"
            id = "x"
            provider = "google"
            identity = "me@gmail.com"
            access_token = "ya29.SECRET"
        "#;
        let err = parse_account(Path::new("/x/x.toml"), toml).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)), "got {err:?}");
    }

    #[test]
    fn an_id_mismatch_is_refused() {
        let toml = r#"
            id = "claimed"
            provider = "google"
            identity = "me@gmail.com"
        "#;
        let err = parse_account(Path::new("/x/actual.toml"), toml).unwrap_err();
        assert!(matches!(err, ConfigError::IdMismatch { .. }));
    }

    #[test]
    fn load_accounts_skips_malformed_and_reads_valid() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("good.toml"),
            "id = \"good\"\nprovider = \"google\"\nidentity = \"a@b.c\"\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("bad.toml"), "id = \"bad\"\nnope =").unwrap();
        std::fs::write(tmp.path().join("ignore.txt"), "not a config").unwrap();

        let (accounts, errors) = load_accounts(tmp.path());
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, "good");
        assert_eq!(errors.len(), 1, "the malformed config is reported, not granted");
        // A missing directory is an empty set, not an error.
        let (none, errs) = load_accounts(&PathBuf::from("/no/such/dir"));
        assert!(none.is_empty() && errs.is_empty());
    }
}
