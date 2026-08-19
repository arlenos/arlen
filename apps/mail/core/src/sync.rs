// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! What a cached mailbox may keep when the server describes itself again.
//!
//! `mail-app.md` section 5 decides the key: sync is keyed on
//! `(identity, UIDVALIDITY, UID)`, `OBJECTID` is used where the server offers
//! it, and name-based identity is a fallback that breaks on RENAME. This module
//! is that decision written as types, because the alternative is the decision
//! living in whichever function was written first.
//!
//! **The two failures this exists to prevent are the ones every mail client has
//! shipped at least once.** A mailbox whose `UIDVALIDITY` changed is a mailbox
//! whose UIDs mean something else now: keeping the cache duplicates every
//! message, or worse, shows the body of one message under the subject of
//! another. And a mailbox identified only by name is a mailbox that vanishes on
//! rename, taking its cached state with it and re-downloading the lot - which is
//! merely slow, and is the price of the fallback rather than a bug in it.
//!
//! Nothing here talks to a server. It answers one question - what may be kept -
//! and that question has a right answer independent of any protocol library.

/// How a server names a mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailboxIdentity {
    /// `OBJECTID` (RFC 8474): a name the server promises to keep across a
    /// rename. Preferred wherever it is offered.
    Object(String),
    /// The mailbox name. The fallback, and it does not survive RENAME - the
    /// same mailbox under a new name is indistinguishable from a new one.
    Name(String),
}

impl MailboxIdentity {
    /// Whether these two names refer to the same mailbox.
    ///
    /// An `OBJECTID` and a name are never comparable: the server that offered
    /// an id and the one that did not are describing the same mailbox in two
    /// vocabularies, and guessing they match is how a cache gets attached to
    /// the wrong folder.
    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Object(a), Self::Object(b)) => a == b,
            (Self::Name(a), Self::Name(b)) => a == b,
            _ => false,
        }
    }
}

/// What a sync knows about a mailbox at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxState {
    /// How the server named it.
    pub identity: MailboxIdentity,
    /// The server's `UIDVALIDITY` for it.
    pub uid_validity: u32,
}

/// One message, keyed the way the plan decided.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MessageKey {
    /// The `UIDVALIDITY` the UID was issued under.
    ///
    /// Carried WITH the UID rather than beside it: a bare UID is only meaningful
    /// against the validity that issued it, and a key that can be compared
    /// across validities is a key that will be.
    pub uid_validity: u32,
    /// The server's UID.
    pub uid: u32,
}

/// What may be kept when the server describes a mailbox again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Continuation {
    /// Same mailbox, same validity: the cached messages still mean what they
    /// meant.
    Keep,
    /// Same mailbox, new `UIDVALIDITY`: every cached UID is meaningless and the
    /// messages must be discarded before anything is fetched.
    ///
    /// Not a merge and not a repair. The server has said its numbering restarted,
    /// and a client that keeps the old rows shows two copies of everything at
    /// best and one message's body under another's subject at worst.
    DiscardMessages,
    /// Not the same mailbox at all.
    ///
    /// Under name-identity this is also what a RENAME looks like, which is the
    /// documented cost of the fallback: the state is dropped and refetched. It
    /// is slow, not wrong - and with `OBJECTID` it does not happen.
    Unrelated,
}

/// What the cache may keep, given what it holds and what the server now says.
#[must_use]
pub fn reconcile(cached: &MailboxState, seen: &MailboxState) -> Continuation {
    if !cached.identity.same(&seen.identity) {
        return Continuation::Unrelated;
    }
    if cached.uid_validity == seen.uid_validity {
        Continuation::Keep
    } else {
        Continuation::DiscardMessages
    }
}

/// Which cached keys are still valid under `state`.
///
/// Filtered rather than assumed: a key from another validity is not a key that
/// needs updating, it is a key about a mailbox numbering that no longer exists.
#[must_use]
pub fn still_valid(keys: &[MessageKey], state: &MailboxState) -> Vec<MessageKey> {
    keys.iter()
        .filter(|k| k.uid_validity == state.uid_validity)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, validity: u32) -> MailboxState {
        MailboxState { identity: MailboxIdentity::Name(name.into()), uid_validity: validity }
    }

    fn object(id: &str, validity: u32) -> MailboxState {
        MailboxState { identity: MailboxIdentity::Object(id.into()), uid_validity: validity }
    }

    #[test]
    fn the_same_mailbox_at_the_same_validity_keeps_what_it_had() {
        assert_eq!(reconcile(&named("INBOX", 7), &named("INBOX", 7)), Continuation::Keep);
    }

    #[test]
    fn a_new_uidvalidity_discards_the_messages_rather_than_merging_them() {
        // The failure this exists for: the server has said its numbering
        // restarted, so every cached UID now names a different message.
        assert_eq!(
            reconcile(&named("INBOX", 7), &named("INBOX", 8)),
            Continuation::DiscardMessages
        );
    }

    #[test]
    fn an_object_id_carries_the_mailbox_through_a_rename() {
        // The whole reason to prefer OBJECTID: the name changed and the cache
        // survives, because the server promised the id would not.
        assert_eq!(reconcile(&object("abc", 7), &object("abc", 7)), Continuation::Keep);
    }

    #[test]
    fn a_rename_under_name_identity_reads_as_a_different_mailbox() {
        // The documented cost of the fallback, asserted rather than left to be
        // discovered: this is slow, not wrong.
        assert_eq!(reconcile(&named("Work", 7), &named("Work Archive", 7)), Continuation::Unrelated);
    }

    #[test]
    fn an_id_and_a_name_are_never_taken_for_each_other() {
        // Two vocabularies for the same mailbox. Guessing they match is how a
        // cache gets attached to the wrong folder.
        assert_eq!(reconcile(&object("abc", 7), &named("INBOX", 7)), Continuation::Unrelated);
    }

    #[test]
    fn keys_from_a_dead_validity_are_dropped_rather_than_renumbered() {
        let keys = vec![
            MessageKey { uid_validity: 7, uid: 1 },
            MessageKey { uid_validity: 7, uid: 2 },
            MessageKey { uid_validity: 8, uid: 1 },
        ];
        let kept = still_valid(&keys, &named("INBOX", 8));
        assert_eq!(kept, vec![MessageKey { uid_validity: 8, uid: 1 }]);
    }

    #[test]
    fn a_uid_alone_cannot_be_compared_across_validities() {
        // Same UID, different numbering, different message. The key carries the
        // validity precisely so this comparison cannot be made by accident.
        let a = MessageKey { uid_validity: 7, uid: 42 };
        let b = MessageKey { uid_validity: 8, uid: 42 };
        assert_ne!(a, b);
    }
}
