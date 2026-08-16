//! Provenance of an agent-writable graph fact: who asserted it
//! (bitemporal-knowledge-graph.md §5.1).
//!
//! A dedicated enum with stable lowercase DB keys, deliberately NOT the
//! `tagging.rs::Origin` prompt-block display strings (`USER-QUESTION`,
//! `GRAPH-DATA`, ...). Those are presentation strings that wrap content for the
//! model, not stable schema keys; coupling a stored column's domain to a
//! display string is a latent footgun (a stored fact is not a "question"). The
//! mapping from a stored `Provenance` back to a prompt `Origin` block is a small
//! pure function on the agent's read/prompt path, where `Origin` is in scope, so
//! it lives there, not here.

/// Who asserted a graph fact. Written to the `origin` column of an
/// agent-writable edge as its stable lowercase DB key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// The user authored or directly confirmed it.
    User,
    /// Promotion derived it from an observed event.
    Graph,
    /// Derived from external content (a parsed document).
    External,
    /// The agent asserted it from model reasoning.
    Model,
    /// The idle curator consolidated it.
    Agent,
}

impl Provenance {
    /// The stable DB key stored in the `origin` column.
    pub fn as_key(self) -> &'static str {
        match self {
            Provenance::User => "user",
            Provenance::Graph => "graph",
            Provenance::External => "external",
            Provenance::Model => "model",
            Provenance::Agent => "agent",
        }
    }

    /// Parse a stored `origin` key. An unknown or absent key yields `None` (fail
    /// closed: a corrupt or legacy value is never silently treated as a trusted
    /// origin; the caller decides how to handle an unknown provenance, e.g. the
    /// governance gate refuses a write driven by a fact of unknown origin).
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "user" => Some(Provenance::User),
            "graph" => Some(Provenance::Graph),
            "external" => Some(Provenance::External),
            "model" => Some(Provenance::Model),
            "agent" => Some(Provenance::Agent),
            _ => None,
        }
    }

    /// The §5.6 protected-origin trust rank: a higher number is more trusted, so
    /// when two devices assert the same membership fact the higher-ranked origin
    /// wins the merge (graph-drift.md §5.6, `user > agent > model > external`).
    /// A `None` means "the trust order does not place this origin", so a caller
    /// must NOT silently rank it - it falls through to the HLC tiebreak or fails
    /// closed rather than guessing.
    ///
    /// `Graph` (promotion-derived) is `None`, and that is now a PENDING DECISION
    /// rather than a hypothetical. This said promotion "sets none" and named the
    /// condition that would change it: "if a future promotion writes
    /// `origin = 'graph'`, its rank must be decided and added HERE". As of 16
    /// August promotion DOES write it - `project::store::link_file` stamps the
    /// bitemporal interval and the origin - so the condition has been met and the
    /// rank is owed.
    ///
    /// It is deliberately still `None`, because ranking a system observation
    /// against the asserted origins is a trust decision and §5.6 does not make
    /// it: whether "the filesystem was observed to contain this" out- or
    /// under-ranks "a model asserted this" changes which side wins a real
    /// conflict. `None` is the safe reading in the meantime - a caller must not
    /// silently rank it, so the merge falls through to the HLC tiebreak or fails
    /// closed rather than guessing, which is exactly what this comment was built
    /// to force.
    ///
    /// This is a pure comparison; it grants no authority until it is wired into
    /// the (executor-live-gated) resolve-membership merge pass, so nothing is
    /// broken while the decision is open.
    pub fn trust_rank(self) -> Option<u8> {
        match self {
            Provenance::User => Some(3),
            Provenance::Agent => Some(2),
            Provenance::Model => Some(1),
            Provenance::External => Some(0),
            Provenance::Graph => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_for_every_variant() {
        for p in [
            Provenance::User,
            Provenance::Graph,
            Provenance::External,
            Provenance::Model,
            Provenance::Agent,
        ] {
            assert_eq!(Provenance::from_key(p.as_key()), Some(p));
        }
    }

    #[test]
    fn keys_are_the_stable_lowercase_db_strings() {
        assert_eq!(Provenance::User.as_key(), "user");
        assert_eq!(Provenance::Graph.as_key(), "graph");
        assert_eq!(Provenance::External.as_key(), "external");
        assert_eq!(Provenance::Model.as_key(), "model");
        assert_eq!(Provenance::Agent.as_key(), "agent");
    }

    #[test]
    fn an_unknown_or_empty_key_fails_closed() {
        assert_eq!(Provenance::from_key(""), None);
        assert_eq!(Provenance::from_key("USER-QUESTION"), None, "not the prompt label");
        assert_eq!(Provenance::from_key("admin"), None);
    }

    #[test]
    fn trust_rank_orders_user_over_agent_over_model_over_external() {
        // The §5.6 order: a higher rank wins the merge tiebreak.
        let u = Provenance::User.trust_rank().unwrap();
        let a = Provenance::Agent.trust_rank().unwrap();
        let m = Provenance::Model.trust_rank().unwrap();
        let e = Provenance::External.trust_rank().unwrap();
        assert!(u > a, "user outranks agent");
        assert!(a > m, "agent outranks model");
        assert!(m > e, "model outranks external");
        // External is the floor of the asserted origins: an untrusted document
        // never wins against a user, agent or model assertion.
        assert!(u > e && a > e && m > e);
    }

    #[test]
    fn graph_origin_is_unranked_so_it_is_never_silently_ranked() {
        // This guarded a hypothetical - "not an edge origin today ... if
        // promotion ever stamps origin='graph'" - and since 16 August promotion
        // DOES stamp it (`project::store::link_file`). So the assertion now
        // covers LIVE data rather than an unused variant, which makes it
        // stronger, not weaker: real edges carry this origin and nothing may
        // rank them by guessing.
        //
        // §5.6 orders the ASSERTED origins (user > agent > model > external) and
        // does not place a system observation among them. Whether being observed
        // out- or under-ranks being asserted decides who wins a real conflict
        // over where a file belongs, so it is a trust decision rather than a
        // coding one. Until it is made, `None` is the honest answer and callers
        // fall through to the HLC tiebreak or fail closed.
        //
        // When somebody DOES decide it, this test is the thing that will fail,
        // which is the point: the rank cannot change quietly.
        assert_eq!(Provenance::Graph.trust_rank(), None);
    }
}
