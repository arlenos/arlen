//! The transfer policy: default-deny, first-match, directional (profile-system-plan.md, Decided 4).
//!
//! Modeled on Qubes' dom0 copy-paste / qrexec policy. Every rule is an ORDERED
//! `(source, dest, type)` triple with an `allow` verdict; [`decide`] scans the
//! rules in order and returns the first match's verdict, and a request that
//! matches no rule is DENIED. There is no implicit-allow path and no synthesized
//! "allow rest" terminal rule: the absence of a match is a deny, full stop.
//!
//! Directional is structural, not a flag. A rule for `work -> personal` says
//! nothing about `personal -> work`; the matcher never reads a rule's pair in
//! reverse. The foundation's `out-only` / `in-only` are two independent directed
//! rules, and a "both" UI choice compiles to two rules. There is no symmetric
//! rule the matcher could mistake for bidirectional.
//!
//! The Locked-profile invariant (transfer hardcoded off in all directions, no
//! config surface) is enforced HERE, before the rule scan: if either side is a
//! Locked profile, [`decide`] returns `Deny` unconditionally, so no policy file
//! can ever express a Locked-allow. The Locked state is carried on the
//! [`ProfileRef`] passed to `decide`, resolved from the profile registry, never
//! a config rule.

use serde::Deserialize;

use crate::request::{ProfileId, TransferType};

/// The decision for one transfer flow. The whole point of the gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The flow is permitted by a matching `allow` rule (and neither side is
    /// Locked).
    Allow,
    /// The flow is refused: no matching rule, a matching `deny` rule, or a
    /// Locked profile on either side.
    Deny,
}

/// One directional policy rule. The `(source, dest)` pair is ORDERED and never
/// read in reverse; `allow == false` is an explicit deny that short-circuits the
/// scan (so a deny can sit before a broader allow).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferRule {
    /// The originating profile (the rule matches only this direction).
    pub source: ProfileId,
    /// The destination profile.
    pub dest: ProfileId,
    /// The flow type this rule governs.
    pub ty: TransferType,
    /// The verdict when this rule matches: `true` permits, `false` denies.
    pub allow: bool,
}

/// The ordered rule table. Order is significant (first-match). An empty table
/// denies everything (default-deny), which is also the safe state a failed
/// config load resolves to.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferPolicy {
    /// The directional rules, scanned in order by [`decide`].
    #[serde(default, rename = "rule")]
    pub rules: Vec<TransferRule>,
}

/// A profile as the gate sees it: its id plus whether it is a Locked profile.
/// The Locked flag is resolved from the profile registry by the daemon and
/// passed in, so the un-overridable Locked-off invariant is a [`decide`]
/// precondition rather than a config rule the policy file could subvert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRef<'a> {
    /// The profile id.
    pub id: &'a ProfileId,
    /// Whether this is a Locked (institution-deployed, no-transfer) profile.
    pub is_locked: bool,
}

impl<'a> ProfileRef<'a> {
    /// A non-Locked profile reference.
    pub fn unlocked(id: &'a ProfileId) -> Self {
        Self {
            id,
            is_locked: false,
        }
    }

    /// A Locked profile reference (transfer hardcoded off in all directions).
    pub fn locked(id: &'a ProfileId) -> Self {
        Self {
            id,
            is_locked: true,
        }
    }
}

/// The transfer decision for `source -> dest` of `ty` under `policy`.
///
/// Fail-closed and order-significant:
/// 1. If EITHER `source` or `dest` is Locked, the verdict is `Deny`
///    unconditionally - the system-level Locked-off invariant, checked before
///    the rules are consulted, so no rule can override it.
/// 2. Otherwise the rules are scanned in order; the FIRST rule whose
///    `(source, dest, ty)` all equal the request's decides it (`allow` -> Allow,
///    else Deny). The pair is matched as an ordered tuple, never in reverse.
/// 3. If no rule matches, the verdict is `Deny` (default-deny). There is no
///    implicit-allow and no synthesized terminal rule.
pub fn decide(
    policy: &TransferPolicy,
    source: &ProfileRef<'_>,
    dest: &ProfileRef<'_>,
    ty: TransferType,
) -> Verdict {
    // The Locked invariant is enforced before any rule is read, so the policy
    // table can never express a Locked-allow.
    if source.is_locked || dest.is_locked {
        return Verdict::Deny;
    }
    for rule in &policy.rules {
        if &rule.source == source.id && &rule.dest == dest.id && rule.ty == ty {
            return if rule.allow {
                Verdict::Allow
            } else {
                Verdict::Deny
            };
        }
    }
    // No rule matched: default-deny.
    Verdict::Deny
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(name: &str) -> ProfileId {
        ProfileId::new(name).expect("valid test profile id")
    }

    fn rule(source: &str, dest: &str, ty: TransferType, allow: bool) -> TransferRule {
        TransferRule {
            source: pid(source),
            dest: pid(dest),
            ty,
            allow,
        }
    }

    #[test]
    fn no_rule_is_deny() {
        // Default-deny: the empty policy refuses everything.
        let policy = TransferPolicy::default();
        let work = pid("work");
        let personal = pid("personal");
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&work),
                &ProfileRef::unlocked(&personal),
                TransferType::Clipboard,
            ),
            Verdict::Deny,
        );
    }

    #[test]
    fn first_match_wins() {
        // An explicit deny before a broader allow short-circuits the scan.
        let policy = TransferPolicy {
            rules: vec![
                rule("work", "personal", TransferType::File, false),
                rule("work", "personal", TransferType::File, true),
            ],
        };
        let work = pid("work");
        let personal = pid("personal");
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&work),
                &ProfileRef::unlocked(&personal),
                TransferType::File,
            ),
            Verdict::Deny,
            "the first matching rule decides, even when a later rule would allow",
        );
    }

    #[test]
    fn direction_is_not_symmetric() {
        // A work->personal allow does NOT permit personal->work.
        let policy = TransferPolicy {
            rules: vec![rule("work", "personal", TransferType::Clipboard, true)],
        };
        let work = pid("work");
        let personal = pid("personal");
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&work),
                &ProfileRef::unlocked(&personal),
                TransferType::Clipboard,
            ),
            Verdict::Allow,
            "the declared direction is allowed",
        );
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&personal),
                &ProfileRef::unlocked(&work),
                TransferType::Clipboard,
            ),
            Verdict::Deny,
            "the reverse direction is not implied by the forward allow",
        );
    }

    #[test]
    fn per_type_is_independent() {
        // A clipboard allow says nothing about file transfer for the same pair.
        let policy = TransferPolicy {
            rules: vec![rule("work", "personal", TransferType::Clipboard, true)],
        };
        let work = pid("work");
        let personal = pid("personal");
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&work),
                &ProfileRef::unlocked(&personal),
                TransferType::Clipboard,
            ),
            Verdict::Allow,
        );
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&work),
                &ProfileRef::unlocked(&personal),
                TransferType::File,
            ),
            Verdict::Deny,
            "the file flow has no rule, so it is denied",
        );
    }

    #[test]
    fn a_locked_profile_is_denied_even_with_an_explicit_allow() {
        // The Locked invariant is un-overridable: an allow rule naming a Locked
        // profile is ignored, the verdict is Deny in both directions.
        let policy = TransferPolicy {
            rules: vec![
                rule("exam", "personal", TransferType::File, true),
                rule("personal", "exam", TransferType::File, true),
            ],
        };
        let exam = pid("exam");
        let personal = pid("personal");
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::locked(&exam),
                &ProfileRef::unlocked(&personal),
                TransferType::File,
            ),
            Verdict::Deny,
            "a Locked source is denied despite the allow rule",
        );
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&personal),
                &ProfileRef::locked(&exam),
                TransferType::File,
            ),
            Verdict::Deny,
            "a Locked dest is denied despite the allow rule",
        );
    }

    #[test]
    fn all_deny_rules_stay_deny() {
        // A policy of only deny rules is deny, and a non-matching pair is also
        // deny - there is no implicit allow anywhere.
        let policy = TransferPolicy {
            rules: vec![rule("work", "personal", TransferType::File, false)],
        };
        let work = pid("work");
        let personal = pid("personal");
        let other = pid("other");
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&work),
                &ProfileRef::unlocked(&personal),
                TransferType::File,
            ),
            Verdict::Deny,
        );
        assert_eq!(
            decide(
                &policy,
                &ProfileRef::unlocked(&work),
                &ProfileRef::unlocked(&other),
                TransferType::File,
            ),
            Verdict::Deny,
            "an unmatched pair is denied, not allowed",
        );
    }

    #[test]
    fn same_profile_follows_the_table() {
        // source == dest is not special-cased; with no rule it is denied.
        let empty = TransferPolicy::default();
        let work = pid("work");
        assert_eq!(
            decide(
                &empty,
                &ProfileRef::unlocked(&work),
                &ProfileRef::unlocked(&work),
                TransferType::Clipboard,
            ),
            Verdict::Deny,
        );
    }
}
