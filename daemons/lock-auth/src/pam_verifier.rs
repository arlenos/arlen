//! The real PAM-backed [`FactorVerifier`] for the password factor
//! (greeter-onboarding-plan.md GR-R1: "the systemd-homed key release from the
//! password"). Feature-gated behind `pam` and built / verified on hardware
//! (like PWR-R2's logind path): it links `libpam` and its behaviour depends on
//! the host PAM stack, so the default build and CI never compile it - the
//! security composition in [`crate::auth`] is what is exhaustively unit-tested.
//!
//! How the home/FDE key is released: this runs a normal PAM authentication for
//! the account against a service whose stack includes `pam_systemd_home`. When
//! that module is in the `auth` stack, a successful authentication derives and
//! installs the systemd-homed LUKS key as part of the auth step - we do not call
//! homed directly, we let the proven PAM stack do it (greeter-onboarding-plan.md:
//! "do not reprogram auth"). The service name is therefore load-bearing and is a
//! constructor parameter; [`PamVerifier::for_homed`] uses the Arlen default.
//!
//! This verifier handles ONLY the password factor. The device factors (FIDO2,
//! fingerprint) verify through their own backends (`pam_u2f` / `libfido2`,
//! `pam_fprintd`); routing those is a follow-up, and until then a non-password
//! presentation fails closed here rather than being silently accepted.

#![cfg(feature = "pam")]

use pam::{Client, PamReturnCode};

use crate::auth::{FactorVerifier, Presentation, VerifyError};
use crate::tier::Factor;

/// The default PAM service for an Arlen home unlock. Its stack must include
/// `pam_systemd_home` so a successful auth releases the systemd-homed key.
pub const DEFAULT_HOMED_SERVICE: &str = "arlen-unlock";

/// A [`FactorVerifier`] that checks a password against a PAM service. A
/// successful check releases the systemd-homed key via the service's
/// `pam_systemd_home` step (see the module doc).
pub struct PamVerifier {
    service: String,
}

impl PamVerifier {
    /// A verifier against an explicit PAM service name.
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    /// A verifier against the Arlen home-unlock service ([`DEFAULT_HOMED_SERVICE`]).
    pub fn for_homed() -> Self {
        Self::new(DEFAULT_HOMED_SERVICE)
    }
}

/// Map a PAM failure return code to a [`VerifyError`].
///
/// Wrong-password-class codes all collapse to [`VerifyError::BadCredential`]
/// with NO distinction between "wrong password" and "no such user"
/// ([`PamReturnCode::User_Unknown`]) - distinguishing them would be a
/// username-enumeration oracle. Everything else (a service/system/conversation
/// fault, an expired account, a required token change) is a backend condition,
/// not a wrong-password event, so it fails closed as [`VerifyError::Backend`]
/// rather than being mistaken for a bad password.
fn map_failure(code: PamReturnCode) -> VerifyError {
    match code {
        PamReturnCode::Auth_Err
        | PamReturnCode::Cred_Insufficient
        | PamReturnCode::Perm_Denied
        | PamReturnCode::MaxTries
        | PamReturnCode::User_Unknown => VerifyError::BadCredential,
        other => VerifyError::Backend(format!("pam: {other:?}")),
    }
}

impl FactorVerifier for PamVerifier {
    fn verify(&self, presentation: &Presentation) -> Result<Factor, VerifyError> {
        let (user, password) = match presentation {
            Presentation::Password { user, password } => (*user, *password),
            // Device factors are verified by their own backends; this verifier
            // refuses them rather than accepting an unchecked factor.
            _ => {
                return Err(VerifyError::Backend(
                    "PamVerifier handles only the password factor".to_string(),
                ))
            }
        };

        let mut client = Client::with_password(&self.service)
            .map_err(|e| VerifyError::Backend(format!("pam: start: {e}")))?;
        client.conversation_mut().set_credentials(user, password);
        match client.authenticate() {
            Ok(()) => Ok(Factor::Password),
            Err(e) => Err(map_failure(e.0)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrong_password_class_codes_map_to_bad_credential_without_a_user_oracle() {
        // A wrong password and a non-existent user must be indistinguishable.
        assert_eq!(map_failure(PamReturnCode::Auth_Err), VerifyError::BadCredential);
        assert_eq!(
            map_failure(PamReturnCode::User_Unknown),
            VerifyError::BadCredential
        );
        assert_eq!(map_failure(PamReturnCode::MaxTries), VerifyError::BadCredential);
        assert_eq!(
            map_failure(PamReturnCode::Perm_Denied),
            VerifyError::BadCredential
        );
    }

    #[test]
    fn service_and_system_faults_fail_closed_as_backend() {
        assert!(matches!(
            map_failure(PamReturnCode::Service_Err),
            VerifyError::Backend(_)
        ));
        assert!(matches!(
            map_failure(PamReturnCode::Conv_Err),
            VerifyError::Backend(_)
        ));
        // An expired account is a backend condition, not a wrong password.
        assert!(matches!(
            map_failure(PamReturnCode::Acct_Expired),
            VerifyError::Backend(_)
        ));
    }
}
