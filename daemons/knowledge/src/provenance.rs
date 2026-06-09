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
}
