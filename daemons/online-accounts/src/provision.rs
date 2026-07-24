//! OA-R2: persist the tokens obtained from the OAuth flow into the vault.
//!
//! The [`crate::flow::authorize`] flow returns a [`TokenResponse`]; provisioning
//! writes it into the per-account [`Vault`] so `GetAccessToken` can hand out the
//! access token and a later refresh can mint a new one without re-running the
//! browser flow.
//!
//! The layout is ADDITIVE and leaves the credential-handout read path untouched:
//! the access token goes in the PRIMARY record (`account_id`), exactly the bare
//! string `GetAccessToken` already reads; the refresh token, when the provider
//! issues one, goes in a sibling `{account_id}.refresh` record that only the
//! refresh flow reads. So a handout still works unchanged and the refresh
//! material is stored separately, never handed to a token consumer.

use crate::oauth::TokenResponse;
use crate::vault::{Vault, VaultError};

/// The sibling vault record id holding an account's refresh token.
pub fn refresh_record_id(account_id: &str) -> String {
    format!("{account_id}.refresh")
}

/// Store the flow's tokens for `account_id`: the access token in the primary
/// record (the handout slot) and, when present, the refresh token in the
/// sibling refresh record. A re-provision overwrites the primary token and, if a
/// new refresh token is issued, the sibling too. Note: a re-provision that
/// yields no refresh token does NOT clear a previously-stored sibling (this path
/// has no delete); the refresh flow is the sole reader of that record.
pub fn provision_tokens(
    vault: &Vault,
    account_id: &str,
    tokens: &TokenResponse,
) -> Result<(), VaultError> {
    vault.store(account_id, tokens.access_token.as_bytes())?;
    if let Some(refresh) = &tokens.refresh_token {
        vault.store(&refresh_record_id(account_id), refresh.as_bytes())?;
    }
    Ok(())
}

/// Read the stored refresh token for `account_id`, if one was provisioned.
pub fn load_refresh_token(vault: &Vault, account_id: &str) -> Result<Option<String>, VaultError> {
    match vault.load(&refresh_record_id(account_id))? {
        Some(bytes) => Ok(String::from_utf8(bytes).ok()),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::parse_token_response;

    fn temp_vault() -> (Vault, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        // A fixed 32-byte master secret for the test store.
        let vault = Vault::new([7u8; 32], dir.path());
        (vault, dir)
    }

    #[test]
    fn provisioning_stores_the_access_token_in_the_handout_slot() {
        let (vault, _dir) = temp_vault();
        let tokens = parse_token_response(
            r#"{"access_token":"at-1","token_type":"Bearer","expires_in":3600,"refresh_token":"rt-1"}"#,
        )
        .unwrap();
        provision_tokens(&vault, "google-alice", &tokens).unwrap();

        // GetAccessToken reads the primary record as a bare string: the access
        // token is exactly there.
        let primary = vault.load("google-alice").unwrap().unwrap();
        assert_eq!(String::from_utf8(primary).unwrap(), "at-1");
        // The refresh token is in the sibling record, never the handout slot.
        assert_eq!(
            load_refresh_token(&vault, "google-alice").unwrap().as_deref(),
            Some("rt-1")
        );
    }

    #[test]
    fn a_provider_without_a_refresh_token_stores_only_the_access_token() {
        let (vault, _dir) = temp_vault();
        let tokens =
            parse_token_response(r#"{"access_token":"at-only","token_type":"Bearer"}"#).unwrap();
        provision_tokens(&vault, "webdav-bob", &tokens).unwrap();
        assert_eq!(
            String::from_utf8(vault.load("webdav-bob").unwrap().unwrap()).unwrap(),
            "at-only"
        );
        assert_eq!(load_refresh_token(&vault, "webdav-bob").unwrap(), None);
    }
}
