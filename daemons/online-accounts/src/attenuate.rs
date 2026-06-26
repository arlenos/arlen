//! CONN-R2: strict monotonic attenuation of derived OAuth tokens. The Connections
//! broker (which online-accounts grows into) hands an app a token scoped to its
//! grant; when the app asks for LESS (a narrower subset), the broker may let it
//! subtract but NEVER add. RFC 8693 standardises the token-exchange wire format but
//! does NOT guarantee the issued token is narrower - that is deployment policy. So we
//! enforce subtract-only here, the same property as ocap / CHERI capability
//! monotonicity and Macaroons' attenuate-never-amplify. This is the pure check that
//! closes GAP-15; the daemon APPLIES it at the `GetAccessToken` handout (the method
//! carries a `requested_scope` argument, an amplification is refused with
//! `AccessDenied` and audited content-free, and the caller is PID-reuse-guarded). The
//! remaining CONN-R2 work - short-lived derived tokens and on-process-exit revocation
//! - needs the proxy-token model (the broker holding a derivable token rather than
//! passing the vault token through), a separate slice.

use std::collections::BTreeSet;

/// Why a requested downscope was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttenuationError {
    /// The request named scopes the grant does not hold - an amplification, which
    /// attenuation forbids. Carries the offending scopes (sorted) for the audit.
    Amplification(Vec<String>),
}

/// Parse a space-delimited OAuth scope string into a normalised set: split on ASCII
/// whitespace, drop empties, dedup. Scope tokens are case-SENSITIVE (RFC 6749), so
/// they are kept verbatim.
fn scope_set(raw: &str) -> BTreeSet<String> {
    raw.split_whitespace().map(str::to_string).collect()
}

/// Render a scope set as a sorted, space-delimited string. `BTreeSet` iterates in
/// sorted order, so the output is deterministic.
fn render(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(" ")
}

/// Attenuate a `granted` scope string to a `requested` one, subtract-only. An EMPTY
/// request means "the full grant" (no narrowing asked) and returns the granted scope
/// normalised. A non-empty request must be a SUBSET of the grant: any requested scope
/// the grant does not hold is an [`AttenuationError::Amplification`]. On success the
/// result is ALWAYS a subset of `granted` (the monotonic invariant), rendered sorted,
/// space-delimited and deduped.
pub fn attenuate(granted: &str, requested: &str) -> Result<String, AttenuationError> {
    let granted_set = scope_set(granted);
    let requested_set = scope_set(requested);

    if requested_set.is_empty() {
        return Ok(render(&granted_set));
    }

    let excess: Vec<String> = requested_set.difference(&granted_set).cloned().collect();
    if !excess.is_empty() {
        return Err(AttenuationError::Amplification(excess));
    }
    Ok(render(&requested_set))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_subset_request_narrows() {
        assert_eq!(
            attenuate("drive.file profile email", "drive.file"),
            Ok("drive.file".to_string())
        );
    }

    #[test]
    fn an_empty_request_takes_the_full_grant() {
        // No narrowing asked -> the whole grant, normalised (sorted, deduped).
        assert_eq!(attenuate("profile drive.file", ""), Ok("drive.file profile".to_string()));
    }

    #[test]
    fn requesting_a_scope_outside_the_grant_is_amplification() {
        assert_eq!(
            attenuate("drive.file", "drive.file admin"),
            Err(AttenuationError::Amplification(vec!["admin".to_string()]))
        );
    }

    #[test]
    fn scope_strings_are_normalised_sorted_and_deduped() {
        assert_eq!(attenuate("b a a", "a b b"), Ok("a b".to_string()));
    }

    #[test]
    fn scopes_are_case_sensitive() {
        // RFC 6749: scope tokens are case-sensitive, so a case mismatch amplifies.
        assert_eq!(
            attenuate("Drive", "drive"),
            Err(AttenuationError::Amplification(vec!["drive".to_string()]))
        );
    }

    #[test]
    fn an_empty_grant_cannot_be_widened() {
        // A grant with no scope admits only the empty (full-grant) request.
        assert_eq!(attenuate("", ""), Ok(String::new()));
        assert_eq!(
            attenuate("", "anything"),
            Err(AttenuationError::Amplification(vec!["anything".to_string()]))
        );
    }

    #[test]
    fn a_successful_result_is_always_a_subset_of_the_grant() {
        // The monotonic invariant, checked directly: every scope the call returns
        // is one the grant held.
        let granted = "a b c d";
        for req in ["", "a", "b d", "a b c d", "c a"] {
            let out = attenuate(granted, req).unwrap();
            let g = scope_set(granted);
            for s in scope_set(&out) {
                assert!(g.contains(&s), "leaked scope {s} from request {req:?}");
            }
        }
    }
}
