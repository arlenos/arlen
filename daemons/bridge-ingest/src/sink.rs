//! The KG-write [`PlanSink`]: each interpreted [`UpsertPlan`] becomes an
//! idempotent entity upsert plus its edges, persisted through an
//! [`EntityWriter`] seam (foreign-app-bridges piece 2, the persisting half).
//!
//! The seam keeps this crate dependency-light and unit-testable: the sink's
//! logic (upsert the node, then create each edge, fail-loud on any error so a
//! partial plan is never silently dropped) is tested against a recording mock,
//! while the real writer - which wraps the os-sdk graph client + a runtime to
//! drive the async `upsert_entity` from the synchronous stdio host - lives in
//! the daemon binary. Origin-tagging is enforced daemon-side by the bridge's
//! attested caller identity (a bridge can only write its own namespace, never a
//! `system.*` fact); the `bridge` name is carried for provenance/logging.

use crate::host::PlanSink;
use crate::interpret::UpsertPlan;
use serde_json::{Map, Value};

use crate::retry::FailureKind;

/// The persistence operations the KG sink performs, both keyed by stable
/// external keys for idempotent re-sync. The real impl wraps the os-sdk graph
/// client (`upsert_entity` + the edge write) and blocks on it from the sync
/// host; tests use a recording mock. An error carries a message for the human
/// and a [`FailureKind`] for the host, never the raw transport error.
pub trait EntityWriter {
    /// Upsert (create-or-strengthen) a node of `qualified_type` keyed by
    /// `external_key`, with `fields`. Idempotent: a re-sync of the same key
    /// updates in place rather than duplicating.
    ///
    /// `origin_ref` is what the row is upstream, when the mapping declared a
    /// field for it. An implementation pairs it with its own run id to stamp
    /// provenance; without one it writes unstamped rather than inventing a ref.
    fn upsert(
        &mut self,
        qualified_type: &str,
        external_key: &str,
        fields: &Map<String, Value>,
        origin_ref: Option<&str>,
    ) -> Result<(), WriteFailure>;

    /// Create an `edge` from the node `(from_type, from_key)` to the node
    /// `(to_type, to_key)`, both addressed by their stable external keys (the
    /// daemon resolves the deterministic ids). Idempotent: a re-sync does not
    /// duplicate the edge.
    fn link(
        &mut self,
        edge: &str,
        from_type: &str,
        from_key: &str,
        to_type: &str,
        to_key: &str,
    ) -> Result<(), WriteFailure>;
}

/// A failed write: what to tell the user, and whether trying again could help.
///
/// The kind is carried rather than derived later on purpose. Only the writer
/// knows why a write failed; by the time an error is a string, deciding whether
/// to retry means matching on its text, which breaks the first time a message is
/// reworded and fails in the direction that keeps retrying a permanently broken
/// bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFailure {
    /// Human-readable, for the log line and the health report.
    pub message: String,
    /// Whether a later attempt could plausibly succeed.
    pub kind: FailureKind,
}

impl WriteFailure {
    /// A failure that could pass later: the daemon was down, the socket blinked.
    pub fn transient(message: impl Into<String>) -> Self {
        Self { message: message.into(), kind: FailureKind::Transient }
    }

    /// A failure that will repeat until something changes: a refused grant, a
    /// mapping the daemon rejects.
    pub fn hard(message: impl Into<String>) -> Self {
        Self { message: message.into(), kind: FailureKind::Hard }
    }
}

impl std::fmt::Display for WriteFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A [`PlanSink`] that persists each plan through an [`EntityWriter`].
#[derive(Debug)]
pub struct KgPlanSink<W: EntityWriter> {
    writer: W,
}

impl<W: EntityWriter> KgPlanSink<W> {
    /// Wrap an [`EntityWriter`] as a plan sink.
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    /// The underlying writer (for the daemon to inspect / reuse).
    pub fn writer(&self) -> &W {
        &self.writer
    }
}

impl<W: EntityWriter> PlanSink for KgPlanSink<W> {
    fn write_plan(&mut self, _bridge: &str, plan: &UpsertPlan) -> Result<(), String> {
        // Upsert the node first; idempotent, so the edges below can reference it.
        self.writer
            .upsert(
                &plan.qualified_type,
                &plan.external_key,
                &plan.fields,
                plan.origin_ref.as_deref(),
            )
            .map_err(|e| e.to_string())?;
        // Then each edge. Both endpoints take this plan's entity type: the
        // bridge.toml link rule names no target type, so the lead case links
        // within one type (note -> note); a future cross-type link adds a
        // `to_type` to the rule. A failure is surfaced (the host keeps the
        // session but reports the message failed) rather than dropping an edge.
        for link in &plan.links {
            self.writer.link(
                &link.edge,
                &plan.qualified_type,
                &link.from_key,
                &plan.qualified_type,
                &link.to_key,
            )
            .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpret::LinkPlan;

    /// A recording mock writer; `fail_on` makes the matching op error so the
    /// fail-loud path is exercised.
    #[derive(Default)]
    struct MockWriter {
        upserts: Vec<(String, String)>,
        links: Vec<(String, String, String)>,
        origin_refs: Vec<Option<String>>,
        fail_upsert: bool,
        fail_link: bool,
    }

    impl EntityWriter for MockWriter {
        fn upsert(
            &mut self,
            qualified_type: &str,
            external_key: &str,
            _fields: &Map<String, Value>,
            origin_ref: Option<&str>,
        ) -> Result<(), WriteFailure> {
            if self.fail_upsert {
                return Err(WriteFailure::hard("upsert refused"));
            }
            self.origin_refs.push(origin_ref.map(str::to_string));
            self.upserts
                .push((qualified_type.to_string(), external_key.to_string()));
            Ok(())
        }

        fn link(
            &mut self,
            edge: &str,
            _from_type: &str,
            from_key: &str,
            _to_type: &str,
            to_key: &str,
        ) -> Result<(), WriteFailure> {
            if self.fail_link {
                return Err(WriteFailure::hard("link refused"));
            }
            self.links
                .push((edge.to_string(), from_key.to_string(), to_key.to_string()));
            Ok(())
        }
    }

    fn plan_with_link() -> UpsertPlan {
        UpsertPlan {
            qualified_type: "md.obsidian.Note".to_string(),
            external_key: "note-1".to_string(),
            fields: Map::new(),
            origin_ref: None,
            links: vec![LinkPlan {
                edge: "LINKS_TO".to_string(),
                from_key: "note-1".to_string(),
                to_key: "note-2".to_string(),
            }],
        }
    }

    #[test]
    fn a_plan_upserts_the_node_then_each_edge() {
        let mut sink = KgPlanSink::new(MockWriter::default());
        sink.write_plan("obsidian", &plan_with_link()).unwrap();
        assert_eq!(
            sink.writer().upserts,
            vec![("md.obsidian.Note".to_string(), "note-1".to_string())]
        );
        assert_eq!(
            sink.writer().links,
            vec![("LINKS_TO".to_string(), "note-1".to_string(), "note-2".to_string())]
        );
    }

    #[test]
    fn a_failed_upsert_stops_before_any_edge() {
        let mut sink = KgPlanSink::new(MockWriter {
            fail_upsert: true,
            ..Default::default()
        });
        let err = sink.write_plan("obsidian", &plan_with_link()).unwrap_err();
        assert_eq!(err, "upsert refused");
        // The node failed, so no edge was attempted.
        assert!(sink.writer().links.is_empty(), "no edge after a failed upsert");
    }

    #[test]
    fn a_failed_edge_is_surfaced_not_swallowed() {
        let mut sink = KgPlanSink::new(MockWriter {
            fail_link: true,
            ..Default::default()
        });
        let err = sink.write_plan("obsidian", &plan_with_link()).unwrap_err();
        assert_eq!(err, "link refused", "an edge failure is surfaced, never dropped");
        // The node upsert still happened (the plan is one-way, idempotent).
        assert_eq!(sink.writer().upserts.len(), 1);
    }

    #[test]
    fn a_plans_origin_ref_reaches_the_writer() {
        // The sink is the only thing between the mapping and the write, so if it
        // drops the ref the daemon has nothing to stamp and the whole feature is
        // silently inert.
        let mut sink = KgPlanSink::new(MockWriter::default());
        let mut plan = plan_with_link();
        plan.origin_ref = Some("sha:deadbeef".to_string());
        sink.write_plan("bridge.test", &plan).unwrap();
        assert_eq!(
            sink.writer().origin_refs,
            vec![Some("sha:deadbeef".to_string())]
        );
    }

    #[test]
    fn a_plan_without_an_origin_ref_writes_unstamped() {
        // Not an error: a mapping that declares no ref field, or a message that
        // omits it, still writes. Only the provenance is absent.
        let mut sink = KgPlanSink::new(MockWriter::default());
        sink.write_plan("bridge.test", &plan_with_link()).unwrap();
        assert_eq!(sink.writer().origin_refs, vec![None]);
    }

    #[test]
    fn a_plan_without_links_just_upserts() {
        let mut sink = KgPlanSink::new(MockWriter::default());
        let plan = UpsertPlan {
            qualified_type: "md.obsidian.Note".to_string(),
            external_key: "note-1".to_string(),
            fields: Map::new(),
            origin_ref: None,
            links: Vec::new(),
        };
        sink.write_plan("obsidian", &plan).unwrap();
        assert_eq!(sink.writer().upserts.len(), 1);
        assert!(sink.writer().links.is_empty());
    }
}
