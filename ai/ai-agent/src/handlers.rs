//! Built-in workflow handlers (the code behind `kind: workflow` behaviours).

use std::collections::BTreeMap;

use crate::engine::{HandlerError, HandlerOutcome, HandlerRegistry, WorkflowHandler};
use crate::gate::ProposedAction;
use crate::seams::{AgentEvent, GraphHandle};

/// The registry of built-in workflow handlers, keyed by the manifest
/// `handler` id. The daemon registers these; third-party handlers are a
/// later, separately-trusted mechanism.
pub fn builtin_handlers() -> HandlerRegistry {
    let mut registry = HandlerRegistry::new();
    registry.insert(
        "auto_tag_by_project".to_string(),
        Box::new(AutoTagByProject) as Box<dyn WorkflowHandler>,
    );
    registry.insert(
        "tag_untagged_files".to_string(),
        Box::new(TagUntaggedFiles) as Box<dyn WorkflowHandler>,
    );
    registry
}

/// `auto-tag-by-project`: tag a newly opened file with the project it
/// belongs to, resolved as the **most specific** project whose root path is
/// a (component-aware) prefix of the file path. If two projects are equally
/// specific the file is genuinely ambiguous, so the behaviour does not guess
/// (design-doc gap G2); it reaches a terminal condition instead.
pub struct AutoTagByProject;

#[async_trait::async_trait]
impl WorkflowHandler for AutoTagByProject {
    async fn run(
        &self,
        event: &AgentEvent,
        graph: &dyn GraphHandle,
    ) -> Result<HandlerOutcome, HandlerError> {
        let Some(path) = event.fields.get("path") else {
            // The event trigger filters on `path`, so this is unreachable in
            // practice; treated as no-op rather than an error.
            return Ok(HandlerOutcome::Terminal("no_path".to_string()));
        };

        let rows = graph
            .query("MATCH (p:Project) RETURN p.id AS id, p.root_path AS root_path")
            .await
            .map_err(|e| HandlerError(e.to_string()))?;

        // Projects whose root is a component-aware prefix of the path, with
        // the prefix length (longer = more specific).
        let mut matches: Vec<(usize, &str)> = rows
            .iter()
            .filter_map(|row| {
                let id = row.get("id")?.as_str()?;
                let root = row.get("root_path")?.as_str()?;
                path_within(path, root).then_some((root.len(), id))
            })
            .collect();

        let Some(max_len) = matches.iter().map(|(len, _)| *len).max() else {
            return Ok(HandlerOutcome::Terminal("no_matching_project".to_string()));
        };
        matches.retain(|(len, _)| *len == max_len);

        let project_id = match matches.as_slice() {
            [(_, id)] => *id,
            // Equally-specific candidates: ambiguous, do not guess (G2).
            _ => return Ok(HandlerOutcome::Terminal("ambiguous_project".to_string())),
        };

        // Propose optimistically; do not read the File node here. `file.opened`
        // is consumed from the Event Bus directly, while the File node is
        // created later by the knowledge promotion pass, so at this point the
        // node may not exist yet, and a handler-side read would either race or
        // permanently miss the file. Validating the File node exists, the file
        // lies under the project root, and no `FILE_PART_OF` edge is already
        // present is the predict-before-act step's job, where it can fail
        // closed at decision time. That step is the next increment; until it is
        // wired, the gate's conservative cap holds, so these operands are
        // carried for it but never lift an execution gate on their own. The
        // `file` operand is the path, which is the File node id by the daemon's
        // keying convention (knowledge `promotion.rs` creates File nodes with
        // `id = path`).
        Ok(HandlerOutcome::Propose(ProposedAction {
            tool: "graph.write".to_string(),
            summary: format!("Tag {path} as part of project {project_id}"),
            arguments: BTreeMap::from([
                ("file".to_string(), path.clone()),
                ("project".to_string(), project_id.to_string()),
            ]),
        }))
    }
}

/// `tag-untagged-files`: the manual (pull-mode) counterpart of
/// `auto-tag-by-project`. Invoked on demand (no `file.opened` trigger), it
/// scans the graph for a File that lies under a known Project's root but
/// carries no live `FILE_PART_OF` edge, and proposes tagging it. Because it
/// works from the graph rather than an event operand, it is driven by
/// `run_skill` and needs no path on the event; because it is manually invoked
/// (not externally triggered) and the proposal is provable (the File and
/// Project already exist and no edge is present), the gate can lift it to a
/// previewed execution rather than holding it for an external-trigger confirm.
/// It proposes the single most-specific match for the first untagged file
/// found and reaches a terminal otherwise; re-invoking proposes the next.
pub struct TagUntaggedFiles;

#[async_trait::async_trait]
impl WorkflowHandler for TagUntaggedFiles {
    async fn run(
        &self,
        _event: &AgentEvent,
        graph: &dyn GraphHandle,
    ) -> Result<HandlerOutcome, HandlerError> {
        // Three plain reads, no subquery, mirroring `auto-tag`'s engine-safe
        // style: the file ids that already carry a LIVE membership edge, every
        // File, and every Project. A file with only a closed (`invalid_at`)
        // edge is treated as untagged - it was moved out and is fair game.
        let tagged_rows = graph
            .query(
                "MATCH (f:File)-[r:FILE_PART_OF]->(:Project) WHERE r.invalid_at IS NULL \
                 RETURN f.id AS id",
            )
            .await
            .map_err(|e| HandlerError(e.to_string()))?;
        let tagged: std::collections::HashSet<&str> = tagged_rows
            .iter()
            .filter_map(|row| row.get("id")?.as_str())
            .collect();

        let file_rows = graph
            .query("MATCH (f:File) RETURN f.id AS id, f.path AS path")
            .await
            .map_err(|e| HandlerError(e.to_string()))?;

        let project_rows = graph
            .query("MATCH (p:Project) RETURN p.id AS id, p.root_path AS root_path")
            .await
            .map_err(|e| HandlerError(e.to_string()))?;

        // The first untagged file with an unambiguous most-specific project.
        for fr in &file_rows {
            let Some(file_id) = fr.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            if tagged.contains(file_id) {
                continue;
            }
            let path = fr.get("path").and_then(|v| v.as_str()).unwrap_or(file_id);

            let mut matches: Vec<(usize, &str)> = project_rows
                .iter()
                .filter_map(|row| {
                    let id = row.get("id")?.as_str()?;
                    let root = row.get("root_path")?.as_str()?;
                    path_within(path, root).then_some((root.len(), id))
                })
                .collect();
            let Some(max_len) = matches.iter().map(|(len, _)| *len).max() else {
                continue; // no project contains this file
            };
            matches.retain(|(len, _)| *len == max_len);
            let project_id = match matches.as_slice() {
                [(_, id)] => *id,
                _ => continue, // equally-specific candidates: ambiguous, skip (G2)
            };

            // The `file` operand is the File node id (the path, by the daemon's
            // keying convention); the predict-before-act step validates the node
            // exists, lies under the project root and has no live edge before the
            // executor writes, so this proposal is provable for a seeded corpus.
            return Ok(HandlerOutcome::Propose(ProposedAction {
                tool: "graph.write".to_string(),
                summary: format!("Tag {path} as part of project {project_id}"),
                arguments: BTreeMap::from([
                    ("file".to_string(), file_id.to_string()),
                    ("project".to_string(), project_id.to_string()),
                ]),
            }));
        }

        Ok(HandlerOutcome::Terminal("no_untagged_file".to_string()))
    }
}

/// Whether `path` lies within the directory `root`, respecting component
/// boundaries: `root` itself or any descendant, but not a sibling whose name
/// merely starts with `root` (e.g. `/a/lib` does not contain `/a/library`).
fn path_within(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return false;
    }
    let root = root.strip_suffix('/').unwrap_or(root);
    path == root || path.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, HashMap};

    use crate::seams::GraphError;

    /// A graph returning canned project rows (or an error). The handler reads
    /// only the project list; the File node and its links are validated later
    /// by the predict-before-act step, not here.
    struct Graph {
        projects: Vec<HashMap<String, serde_json::Value>>,
        err: bool,
    }

    #[async_trait::async_trait]
    impl GraphHandle for Graph {
        async fn query(
            &self,
            _cypher: &str,
        ) -> Result<Vec<HashMap<String, serde_json::Value>>, GraphError> {
            if self.err {
                return Err(GraphError::Failed("boom".to_string()));
            }
            Ok(self.projects.clone())
        }
    }

    fn projects(pairs: &[(&str, &str)]) -> Graph {
        let rows = pairs
            .iter()
            .map(|(id, root)| {
                HashMap::from([
                    ("id".to_string(), serde_json::Value::from(*id)),
                    ("root_path".to_string(), serde_json::Value::from(*root)),
                ])
            })
            .collect();
        Graph {
            projects: rows,
            err: false,
        }
    }

    fn opened(path: &str) -> AgentEvent {
        AgentEvent {
            id: "e1".to_string(),
            event_type: "file.opened".to_string(),
            fields: BTreeMap::from([("path".to_string(), path.to_string())]),
            external_content: false,
        }
    }

    async fn run(graph: &Graph, path: &str) -> HandlerOutcome {
        AutoTagByProject.run(&opened(path), graph).await.unwrap()
    }

    const ARLEN: &str = "~/Repositories/arlenos";
    const ARLEN_FILE: &str = "~/Repositories/arlenos/foo.rs";

    #[test]
    fn path_within_respects_component_boundaries() {
        assert!(path_within("/a/proj/foo.rs", "/a/proj"));
        assert!(path_within("/a/proj", "/a/proj")); // the root itself
        assert!(path_within("/a/proj/foo.rs", "/a/proj/")); // trailing slash on root
        assert!(!path_within("/a/project/foo.rs", "/a/proj")); // sibling prefix, not contained
        assert!(!path_within("/b/foo.rs", "/a/proj"));
    }

    #[tokio::test]
    async fn proposes_the_matching_project_with_operands() {
        let g = projects(&[("proj-a", ARLEN), ("proj-b", "~/Other")]);
        match run(&g, ARLEN_FILE).await {
            HandlerOutcome::Propose(action) => {
                assert_eq!(action.tool, "graph.write");
                assert!(action.summary.contains("proj-a"));
                // The file operand is the path (the File node id by convention);
                // the project operand is the matched project id.
                assert_eq!(action.arguments.get("file").map(String::as_str), Some(ARLEN_FILE));
                assert_eq!(action.arguments.get("project").map(String::as_str), Some("proj-a"));
            }
            other => panic!("expected a proposal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn most_specific_nested_project_wins() {
        let g = projects(&[
            ("outer", ARLEN),
            ("inner", "~/Repositories/arlenos/desktop-shell"),
        ]);
        match run(&g, "~/Repositories/arlenos/desktop-shell/src/x.rs").await {
            HandlerOutcome::Propose(action) => {
                assert!(action.summary.contains("inner"));
                assert_eq!(action.arguments.get("project").map(String::as_str), Some("inner"));
            }
            other => panic!("expected the inner project, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn no_match_and_ambiguous_reach_terminals() {
        let none = projects(&[("proj-a", ARLEN)]);
        assert!(matches!(
            run(&none, "~/Downloads/x.pdf").await,
            HandlerOutcome::Terminal(t) if t == "no_matching_project"
        ));

        // Two projects claiming the same root: ambiguous, do not guess.
        let tie = projects(&[("a", "~/shared"), ("b", "~/shared")]);
        assert!(matches!(
            run(&tie, "~/shared/x.rs").await,
            HandlerOutcome::Terminal(t) if t == "ambiguous_project"
        ));
    }

    #[tokio::test]
    async fn a_graph_error_propagates_as_handler_error() {
        let g = Graph {
            projects: vec![],
            err: true,
        };
        let err = AutoTagByProject.run(&opened(ARLEN_FILE), &g).await.unwrap_err();
        assert!(format!("{err}").contains("boom"));
    }

    /// A graph for the `tag-untagged-files` handler: it issues three distinct
    /// reads (live-tagged ids, all files, all projects), so the mock routes by
    /// query content rather than returning one canned set.
    struct ScanGraph {
        tagged: Vec<&'static str>,
        files: Vec<(&'static str, &'static str)>,
        projects: Vec<(&'static str, &'static str)>,
        err: bool,
    }

    #[async_trait::async_trait]
    impl GraphHandle for ScanGraph {
        async fn query(
            &self,
            cypher: &str,
        ) -> Result<Vec<HashMap<String, serde_json::Value>>, GraphError> {
            if self.err {
                return Err(GraphError::Failed("boom".to_string()));
            }
            if cypher.contains("FILE_PART_OF") {
                Ok(self
                    .tagged
                    .iter()
                    .map(|id| HashMap::from([("id".to_string(), serde_json::Value::from(*id))]))
                    .collect())
            } else if cypher.contains(":Project)") {
                Ok(self
                    .projects
                    .iter()
                    .map(|(id, root)| {
                        HashMap::from([
                            ("id".to_string(), serde_json::Value::from(*id)),
                            ("root_path".to_string(), serde_json::Value::from(*root)),
                        ])
                    })
                    .collect())
            } else {
                Ok(self
                    .files
                    .iter()
                    .map(|(id, path)| {
                        HashMap::from([
                            ("id".to_string(), serde_json::Value::from(*id)),
                            ("path".to_string(), serde_json::Value::from(*path)),
                        ])
                    })
                    .collect())
            }
        }
    }

    fn invoke() -> AgentEvent {
        AgentEvent {
            id: "m1".to_string(),
            event_type: "manual.invoke".to_string(),
            fields: BTreeMap::new(),
            external_content: false,
        }
    }

    #[tokio::test]
    async fn tag_untagged_proposes_the_first_untagged_file_under_its_project() {
        // One file is already tagged (skipped), one is untagged and under a project.
        let untagged = "~/Repositories/arlenos/new.rs";
        let g = ScanGraph {
            tagged: vec![ARLEN_FILE],
            files: vec![(ARLEN_FILE, ARLEN_FILE), (untagged, untagged)],
            projects: vec![("proj-a", ARLEN)],
            err: false,
        };
        match TagUntaggedFiles.run(&invoke(), &g).await.unwrap() {
            HandlerOutcome::Propose(action) => {
                assert_eq!(action.tool, "graph.write");
                assert_eq!(action.arguments.get("file").map(String::as_str), Some(untagged));
                assert_eq!(action.arguments.get("project").map(String::as_str), Some("proj-a"));
            }
            other => panic!("expected a proposal for the untagged file, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tag_untagged_reaches_terminal_when_all_files_are_tagged() {
        let g = ScanGraph {
            tagged: vec![ARLEN_FILE],
            files: vec![(ARLEN_FILE, ARLEN_FILE)],
            projects: vec![("proj-a", ARLEN)],
            err: false,
        };
        assert!(matches!(
            TagUntaggedFiles.run(&invoke(), &g).await.unwrap(),
            HandlerOutcome::Terminal(t) if t == "no_untagged_file"
        ));
    }

    #[tokio::test]
    async fn tag_untagged_skips_a_file_under_no_project() {
        // Untagged but outside every project root: not proposable, terminal.
        let g = ScanGraph {
            tagged: vec![],
            files: vec![("~/Downloads/x.pdf", "~/Downloads/x.pdf")],
            projects: vec![("proj-a", ARLEN)],
            err: false,
        };
        assert!(matches!(
            TagUntaggedFiles.run(&invoke(), &g).await.unwrap(),
            HandlerOutcome::Terminal(t) if t == "no_untagged_file"
        ));
    }

    #[tokio::test]
    async fn tag_untagged_propagates_a_graph_error() {
        let g = ScanGraph {
            tagged: vec![],
            files: vec![],
            projects: vec![],
            err: true,
        };
        let err = TagUntaggedFiles.run(&invoke(), &g).await.unwrap_err();
        assert!(format!("{err}").contains("boom"));
    }
}
