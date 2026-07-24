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

use crate::flow::TokenExchanger;
use crate::oauth::{refresh_form, TokenResponse};
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

/// A refresh failure.
#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    /// A vault read/write error.
    #[error("vault: {0}")]
    Vault(#[from] VaultError),
    /// No refresh token was stored for this account (it was never provisioned
    /// with one, so it cannot be refreshed without re-running the browser flow).
    #[error("no refresh token stored for the account")]
    NoRefreshToken,
    /// The token endpoint refused or the exchange failed (e.g. the refresh token
    /// was revoked -> RFC 6749 `invalid_grant`).
    #[error("token refresh: {0}")]
    Exchange(String),
}

/// Refresh an account's access token without the browser flow: load its stored
/// refresh token, exchange it at the provider's `token_endpoint`, and
/// re-provision the vault with the new token set (RFC 6749 §6). A provider that
/// rotates the refresh token returns a new one, which [`provision_tokens`] stores
/// over the old; a provider that omits it leaves the existing refresh token in
/// place (so a subsequent refresh still works). The exchange is behind the same
/// [`TokenExchanger`] seam the initial flow uses, so this is mock-testable and
/// the only client-ID-gated part is the live provider round-trip.
pub fn refresh_account(
    vault: &Vault,
    exchanger: &dyn TokenExchanger,
    token_endpoint: &str,
    account_id: &str,
    client_id: &str,
) -> Result<(), RefreshError> {
    let refresh = load_refresh_token(vault, account_id)?.ok_or(RefreshError::NoRefreshToken)?;
    let form = refresh_form(&refresh, client_id);
    let tokens = exchanger
        .exchange(token_endpoint, &form)
        .map_err(RefreshError::Exchange)?;
    // A refresh response may omit refresh_token (the old one stays valid); carry
    // the existing one forward so the account is still refreshable next time.
    let tokens = carry_refresh_token(tokens, refresh);
    provision_tokens(vault, account_id, &tokens)?;
    Ok(())
}

/// If the refresh response omitted a new refresh token, keep the one we used, so
/// the re-provision does not drop the account's ability to refresh again.
fn carry_refresh_token(mut tokens: TokenResponse, used: String) -> TokenResponse {
    if tokens.refresh_token.is_none() {
        tokens.refresh_token = Some(used);
    }
    tokens
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

    struct CannedExchanger {
        response: Result<TokenResponse, String>,
        seen_form: std::cell::RefCell<Option<String>>,
    }
    impl TokenExchanger for CannedExchanger {
        fn exchange(&self, _endpoint: &str, form: &str) -> Result<TokenResponse, String> {
            *self.seen_form.borrow_mut() = Some(form.to_string());
            self.response.clone()
        }
    }

    #[test]
    fn refresh_uses_the_stored_token_and_re_provisions() {
        let (vault, _dir) = temp_vault();
        // Seed an account that has a refresh token.
        let initial = parse_token_response(
            r#"{"access_token":"at-old","token_type":"Bearer","refresh_token":"rt-1"}"#,
        )
        .unwrap();
        provision_tokens(&vault, "google-alice", &initial).unwrap();

        // The provider rotates both tokens.
        let ex = CannedExchanger {
            response: parse_token_response(
                r#"{"access_token":"at-new","token_type":"Bearer","refresh_token":"rt-2"}"#,
            )
            .map_err(|e| e.to_string()),
            seen_form: std::cell::RefCell::new(None),
        };
        refresh_account(&vault, &ex, "https://t/token", "google-alice", "cid").unwrap();

        // The form carried the stored refresh token + the refresh grant.
        let form = ex.seen_form.borrow().clone().unwrap();
        assert!(form.contains("grant_type=refresh_token"));
        assert!(form.contains("&refresh_token=rt-1"));
        // The vault now holds the rotated tokens.
        assert_eq!(
            String::from_utf8(vault.load("google-alice").unwrap().unwrap()).unwrap(),
            "at-new"
        );
        assert_eq!(
            load_refresh_token(&vault, "google-alice").unwrap().as_deref(),
            Some("rt-2")
        );
    }

    #[test]
    fn a_refresh_response_without_a_new_token_keeps_the_old_refresh_token() {
        let (vault, _dir) = temp_vault();
        let initial = parse_token_response(
            r#"{"access_token":"at-old","token_type":"Bearer","refresh_token":"rt-keep"}"#,
        )
        .unwrap();
        provision_tokens(&vault, "webdav-bob", &initial).unwrap();

        // The provider returns a new access token but NO refresh token.
        let ex = CannedExchanger {
            response: parse_token_response(r#"{"access_token":"at-new","token_type":"Bearer"}"#)
                .map_err(|e| e.to_string()),
            seen_form: std::cell::RefCell::new(None),
        };
        refresh_account(&vault, &ex, "https://t/token", "webdav-bob", "cid").unwrap();

        assert_eq!(
            String::from_utf8(vault.load("webdav-bob").unwrap().unwrap()).unwrap(),
            "at-new"
        );
        // The original refresh token is carried forward, still refreshable.
        assert_eq!(
            load_refresh_token(&vault, "webdav-bob").unwrap().as_deref(),
            Some("rt-keep")
        );
    }

    #[test]
    fn refreshing_an_account_with_no_refresh_token_errors() {
        let (vault, _dir) = temp_vault();
        let ex = CannedExchanger {
            response: Err("unused".into()),
            seen_form: std::cell::RefCell::new(None),
        };
        match refresh_account(&vault, &ex, "https://t/token", "absent", "cid") {
            Err(RefreshError::NoRefreshToken) => {}
            other => panic!("expected NoRefreshToken, got {other:?}"),
        }
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
