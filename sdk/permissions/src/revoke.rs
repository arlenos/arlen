//! The capability-revoke wire contract (living-capability-graph.md §6).
//!
//! Revoke is profile-first and narrowing-only: removing a reach can never grant
//! authority. These are the request + outcome types shared across the wire by
//! the knowledge daemon (which deserializes a request, applies the strict-subset
//! narrowing, and replies with an outcome token) and any client (Settings, via
//! `os-sdk`) that constructs the request. The closed [`RevokedReach`] has no
//! variant that adds a reach, so a request cannot express a widening.
//!
//! The daemon-internal logic (the strict-subset gate, the `toml_edit` narrowing)
//! lives in the knowledge daemon; only this wire vocabulary is shared, so the
//! request shape and the outcome tokens have one definition and cannot drift
//! between the daemon and its callers.

use serde::{Deserialize, Serialize};

/// A single reach to remove. A closed enum: there is no variant that *adds* a
/// reach, so a revoke request cannot widen authority by construction (§6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevokedReach {
    /// Remove an entry of `[graph].read` (an entity-type read pattern).
    Read {
        /// The read pattern to remove.
        entity_pattern: String,
    },
    /// Remove an entry of `[graph].write`.
    Write {
        /// The write pattern to remove.
        entity_pattern: String,
    },
    /// Remove a `[graph].relations` entry (a permitted relation creation).
    Relation {
        /// The relation's source entity type.
        from: String,
        /// The relation's target entity type.
        to: String,
        /// The relation type.
        relation_type: String,
    },
    /// Demote `instance_scope` from `All` to `Own` (the app loses cross-app reach).
    InstanceAll,
    /// Remove a domain from `[network].allowed_domains` (the app loses egress to it).
    /// Narrowing-only, like the graph variants: it can only remove an allowed
    /// domain, never add one, and while `allow_all` is set the domain list is moot
    /// so removing an entry is not a narrowing (the gate refuses it).
    NetworkDomain {
        /// The domain to remove from the network egress allowlist.
        domain: String,
    },
    /// Turn off a `[clipboard]` capability flag (`read`/`write`/`read_sensitive`/
    /// `history`). Narrowing-only: it can only disable a capability, never enable
    /// one; the gate proves the enabled-capability set strictly shrank.
    ClipboardCap {
        /// The clipboard capability flag to disable.
        cap: String,
    },
    /// Turn off `[notifications].enabled` (the app can no longer post
    /// notifications). Narrowing-only single-flag dimension.
    NotificationsOff,
    /// Turn off an `[input]` capability flag (focused/global keybinding
    /// registration). Narrowing-only, like the clipboard capabilities.
    InputCap {
        /// The input capability flag to disable.
        cap: String,
    },
    /// Turn off a `[search]` capability flag (open/register_handler/intercept_all).
    SearchCap {
        /// The search capability flag to disable.
        cap: String,
    },
    /// Turn off an `[intents]` capability flag (dispatch/register/preferences).
    IntentsCap {
        /// The intents capability flag to disable.
        cap: String,
    },
    /// Turn off a standard `[filesystem]` directory flag (home/documents/...).
    FilesystemDir {
        /// The directory flag to disable.
        dir: String,
    },
    /// Remove a `[filesystem].custom` path entry (the app loses access to it).
    FilesystemPath {
        /// The custom path to remove.
        path: String,
    },
    /// Remove an `[event_bus].subscribe` pattern (the app hears fewer events).
    EventBusSubscribe {
        /// The subscribe pattern to remove.
        pattern: String,
    },
    /// Remove an `[event_bus].publish` pattern.
    EventBusPublish {
        /// The publish pattern to remove.
        pattern: String,
    },
    /// Turn off a `[system]` capability (`autostart`/`background`, or the nested
    /// `[system.power]` `suspend`/`set_profile`).
    SystemCap {
        /// The system capability flag to disable.
        cap: String,
    },
}

/// Who initiated the revoke. The agent may only *propose* (suggest-mode records a
/// proposal into the pull-review timeline); the user confirming replays it as
/// [`RevokeInitiator::User`]. There is no agent path that auto-applies (§6.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevokeInitiator {
    /// A user-confirmed revoke.
    User,
    /// An agent suggestion, carrying the proposal id it replays.
    Agent {
        /// The suggestion id this revoke replays.
        suggestion_id: String,
    },
}

/// A revoke request. The closed [`RevokedReach`] makes widening unexpressible.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokeReach {
    /// The app whose reach is narrowed.
    pub target_app_id: String,
    /// The reach to remove.
    pub reach: RevokedReach,
    /// Who initiated it.
    pub initiator: RevokeInitiator,
}

/// A restore request: re-add a reach the user previously revoked. The reach is a
/// [`RevokedReach`] (the same closed vocabulary), so a restore can only name a
/// reach that revoke could remove; the daemon additionally bounds it to a reach
/// its removal ledger records, so a restore can only *un-do* a specific prior
/// revoke, never grant fresh authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreReach {
    /// The app whose reach is re-widened.
    pub target_app_id: String,
    /// The reach to re-add (a reach the user previously revoked).
    pub reach: RevokedReach,
    /// Who initiated it. As with revoke, an agent may only *propose*; the daemon
    /// refuses an `Agent` initiator arriving at the apply site (§6.3).
    pub initiator: RevokeInitiator,
}

/// The outcome of a revoke. The daemon maps this to a wire token; a client maps
/// the token back. Both sides use [`RevokeOutcome::wire_token`] /
/// [`RevokeOutcome::from_wire_token`] so the four outcome strings have one
/// definition and cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeOutcome {
    /// The profile was narrowed and written.
    Revoked,
    /// The reach was already absent; nothing changed, nothing written.
    NoChange,
    /// The narrowing did not strictly shrink authority (the gate refused);
    /// nothing written.
    NotNarrowing,
    /// No profile file exists for the target app; nothing to narrow.
    NotFound,
}

impl RevokeOutcome {
    /// The wire token the daemon sends for this outcome (always `OK:`-prefixed,
    /// since these are successful processings, not errors).
    pub fn wire_token(self) -> &'static str {
        match self {
            RevokeOutcome::Revoked => "OK: revoked",
            RevokeOutcome::NoChange => "OK: no-change",
            RevokeOutcome::NotNarrowing => "OK: not-narrowing",
            RevokeOutcome::NotFound => "OK: not-found",
        }
    }

    /// Parse a wire token back to an outcome, or `None` if it is not a recognised
    /// outcome token (an `ERROR:` reply, or an unknown string).
    pub fn from_wire_token(token: &str) -> Option<RevokeOutcome> {
        match token.trim() {
            "OK: revoked" => Some(RevokeOutcome::Revoked),
            "OK: no-change" => Some(RevokeOutcome::NoChange),
            "OK: not-narrowing" => Some(RevokeOutcome::NotNarrowing),
            "OK: not-found" => Some(RevokeOutcome::NotFound),
            _ => None,
        }
    }
}

/// The outcome of a restore (re-widen), the reverse of [`RevokeOutcome`]. Restore
/// is the one authority-growth path: a restore re-adds a reach the user removed,
/// bounded by the app's declared ceiling. Same one-definition wire-token discipline
/// so the daemon and a client cannot drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The reach was re-added and the profile written.
    Restored,
    /// The reach was already present; nothing changed, nothing written.
    NoChange,
    /// The re-widen was refused by the safety gate: it did not strictly widen, or
    /// the result fell outside the app's declared ceiling. Nothing written.
    NotPermitted,
    /// No profile file exists for the target app; nothing to restore.
    NotFound,
}

impl RestoreOutcome {
    /// The wire token the daemon sends for this outcome (always `OK:`-prefixed;
    /// a refusal is a successful, safe processing, not an error).
    pub fn wire_token(self) -> &'static str {
        match self {
            RestoreOutcome::Restored => "OK: restored",
            RestoreOutcome::NoChange => "OK: no-change",
            RestoreOutcome::NotPermitted => "OK: not-permitted",
            RestoreOutcome::NotFound => "OK: not-found",
        }
    }

    /// Parse a wire token back to an outcome, or `None` if it is not a recognised
    /// outcome token (an `ERROR:` reply, or an unknown string).
    pub fn from_wire_token(token: &str) -> Option<RestoreOutcome> {
        match token.trim() {
            "OK: restored" => Some(RestoreOutcome::Restored),
            "OK: no-change" => Some(RestoreOutcome::NoChange),
            "OK: not-permitted" => Some(RestoreOutcome::NotPermitted),
            "OK: not-found" => Some(RestoreOutcome::NotFound),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_tokens_round_trip() {
        for o in [
            RevokeOutcome::Revoked,
            RevokeOutcome::NoChange,
            RevokeOutcome::NotNarrowing,
            RevokeOutcome::NotFound,
        ] {
            assert_eq!(RevokeOutcome::from_wire_token(o.wire_token()), Some(o));
        }
        assert_eq!(RevokeOutcome::from_wire_token("ERROR: nope"), None);
        assert_eq!(RevokeOutcome::from_wire_token("OK: revoked\n"), Some(RevokeOutcome::Revoked));
    }

    #[test]
    fn request_round_trips_through_json() {
        let req = RevokeReach {
            target_app_id: "com.x".into(),
            reach: RevokedReach::Read { entity_pattern: "system.File".into() },
            initiator: RevokeInitiator::User,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(serde_json::from_str::<RevokeReach>(&json).unwrap(), req);
    }

    #[test]
    fn restore_request_round_trips_through_json() {
        let req = RestoreReach {
            target_app_id: "com.x".into(),
            reach: RevokedReach::Read { entity_pattern: "system.File".into() },
            initiator: RevokeInitiator::User,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert_eq!(serde_json::from_str::<RestoreReach>(&json).unwrap(), req);
    }
}
