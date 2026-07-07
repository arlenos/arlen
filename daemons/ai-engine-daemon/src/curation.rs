//! Deterministic curation (`pi-agent-adoption.md` §E): the safe, reversible,
//! zero-token graph curation the daemon runs DIRECTLY for a `kind: workflow`
//! behaviour - no pi, no model, so no external-content injection surface (a
//! workflow makes no model call, which is exactly why the gate's
//! `deterministic_workflow` carve-out lets an externally-triggered one skip
//! always-confirm).
//!
//! The first is `auto-tag-by-project`: tag a newly-opened file with the project
//! it belongs to. This module holds the PURE decision - which project, or
//! none/ambiguous - re-homed from the native agent's handler. Applying the
//! resulting FILE_PART_OF write through the daemon's gated write path (so a real
//! write still needs `executor_live`, is audited, and registers its undo) is a
//! later increment.

use crate::dispatch::Executor;
use crate::session::SessionGrant;
use ai_engine_contract::{CapabilityContext, Execute, ExecuteOutcome, ReadTier};
use std::future::Future;

/// Reads the project rows (`project_id`, `root_path`) the auto-tag decision needs.
/// The production impl reads the daemon's own graph; tests inject a mock.
pub trait ProjectReader {
    /// Every project's id and root path.
    fn projects(&self) -> impl Future<Output = Result<Vec<(String, String)>, String>> + Send;
}

/// The outcome of an auto-tag curation pass (for logging; the write itself is
/// silent to the user).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoTagResult {
    /// The file resolved to `project` and the FILE_PART_OF write was applied;
    /// `written` is whether the daemon's gated write path actually wrote it (false
    /// when executor-live is off, so it stayed a no-op proposal).
    Applied { project: String, written: bool },
    /// No project root is a prefix of the path - nothing to tag.
    NoProject,
    /// Equally-specific projects match - ambiguous, so no guess (G2).
    Ambiguous,
    /// The project read failed; the pass is skipped (never a wrong tag).
    ReadError(String),
}

/// The internal grant a daemon-direct curation write runs under. The write
/// executor ignores the grant (it gates on `executor_live` itself and audits from
/// its own minted op id), so this is a minimal placeholder; the deterministic
/// curation carries no engine session and mints no HIGH-1 proof (that binds an
/// UNtrusted engine's Authorize to its Execute - here the daemon builds the write
/// itself, so there is no engine to distrust).
fn curation_grant() -> SessionGrant {
    SessionGrant {
        capability_context: CapabilityContext { generic_tools: vec![], proxy_tools: vec![] },
        project_anchor: None,
        read_tier: ReadTier::None,
        // The trigger is external (a file.opened), but this write is a
        // DETERMINISTIC workflow with no model in the loop, so external content
        // has nothing to inject into (the gate's deterministic_workflow carve-out).
        externally_triggered: true,
        pid: std::process::id(),
    }
}

/// Run the deterministic auto-tag curation for a newly-opened file: read the
/// projects, resolve the file's project, and - if unambiguous - apply the
/// FILE_PART_OF write DIRECTLY through the daemon's gated write executor (no pi, no
/// model, no HIGH-1 proof). The write executor still enforces `executor_live`,
/// audits before applying, and registers the op-id-keyed undo, so a real write is
/// gated + reversible. No-match / ambiguous / a read error is a silent no-op.
pub async fn run_auto_tag<R: ProjectReader>(
    path: &str,
    reader: &R,
    writer: &dyn Executor,
) -> AutoTagResult {
    let projects = match reader.projects().await {
        Ok(p) => p,
        Err(e) => return AutoTagResult::ReadError(e),
    };
    match resolve_auto_tag(path, &projects) {
        AutoTag::Tag { file, project } => {
            let execute = Execute {
                tool_name: "graph.write".to_string(),
                tool_input: serde_json::json!({
                    "from_type": "File",
                    "from_id": file,
                    "to_type": "Project",
                    "to_id": project,
                    "relation_type": "FILE_PART_OF",
                }),
                proof: None,
            };
            let written = matches!(
                writer.execute(&execute, &curation_grant()).await,
                ExecuteOutcome::Ok { .. }
            );
            AutoTagResult::Applied { project, written }
        }
        AutoTag::NoProject => AutoTagResult::NoProject,
        AutoTag::Ambiguous => AutoTagResult::Ambiguous,
    }
}

/// The auto-tag decision for one file path against the known projects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoTag {
    /// Tag the file as part of this project: the FILE_PART_OF write to apply
    /// (`file` is the File node id = the path, `project` the Project node id).
    Tag { file: String, project: String },
    /// No project root is a component-aware prefix of the path.
    NoProject,
    /// Two or more EQUALLY-specific projects match: genuinely ambiguous, so the
    /// deterministic curation does not guess (design-doc gap G2).
    Ambiguous,
}

/// Whether `path` lies within `root`, respecting component boundaries (so
/// `/a/foo` is NOT within `/a/foobar`). An empty root never matches.
pub fn path_within(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return false;
    }
    let root = root.strip_suffix('/').unwrap_or(root);
    path == root || path.strip_prefix(root).is_some_and(|rest| rest.starts_with('/'))
}

/// Resolve the file's project: the MOST-SPECIFIC project whose root is a
/// component-aware prefix of `path` (longer root = more specific). Two
/// equally-specific matches are ambiguous (no guess); none is `NoProject`.
/// `projects` is `(project_id, root_path)`.
pub fn resolve_auto_tag(path: &str, projects: &[(String, String)]) -> AutoTag {
    let mut matches: Vec<(usize, &str)> = projects
        .iter()
        .filter_map(|(id, root)| path_within(path, root).then_some((root.len(), id.as_str())))
        .collect();
    let Some(max_len) = matches.iter().map(|(len, _)| *len).max() else {
        return AutoTag::NoProject;
    };
    matches.retain(|(len, _)| *len == max_len);
    match matches.as_slice() {
        [(_, id)] => AutoTag::Tag { file: path.to_string(), project: id.to_string() },
        // Equally-specific candidates: ambiguous, do not guess (G2).
        _ => AutoTag::Ambiguous,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projects(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs.iter().map(|(id, root)| (id.to_string(), root.to_string())).collect()
    }

    #[test]
    fn path_within_respects_component_boundaries() {
        assert!(path_within("/home/a/foo/x.rs", "/home/a/foo"));
        assert!(path_within("/home/a/foo/x.rs", "/home/a/foo/")); // trailing slash
        assert!(path_within("/home/a/foo", "/home/a/foo")); // the root itself
        // Component boundary: /foo is NOT within /foobar.
        assert!(!path_within("/home/a/foobar/x.rs", "/home/a/foo"));
        assert!(!path_within("/home/a/x.rs", "/home/a/foo")); // outside
        assert!(!path_within("/home/a/foo/x.rs", "")); // empty root
    }

    #[test]
    fn resolve_auto_tag_picks_the_most_specific_project() {
        // Nested projects: the deeper root wins.
        let ps = projects(&[("outer", "/home/a"), ("inner", "/home/a/foo")]);
        assert_eq!(
            resolve_auto_tag("/home/a/foo/x.rs", &ps),
            AutoTag::Tag { file: "/home/a/foo/x.rs".to_string(), project: "inner".to_string() }
        );
        // A file only under the outer project tags to it.
        assert_eq!(
            resolve_auto_tag("/home/a/bar.rs", &ps),
            AutoTag::Tag { file: "/home/a/bar.rs".to_string(), project: "outer".to_string() }
        );
    }

    #[test]
    fn resolve_auto_tag_no_match_and_ambiguous() {
        let ps = projects(&[("p1", "/home/a"), ("p2", "/home/b")]);
        assert_eq!(resolve_auto_tag("/home/c/x.rs", &ps), AutoTag::NoProject);
        // Two projects with the SAME root are equally specific -> ambiguous.
        let same = projects(&[("p1", "/home/a"), ("p2", "/home/a")]);
        assert_eq!(resolve_auto_tag("/home/a/x.rs", &same), AutoTag::Ambiguous);
    }

    use ai_engine_contract::ContractError;

    struct MockReader {
        projects: Vec<(String, String)>,
        err: Option<String>,
    }
    impl ProjectReader for MockReader {
        async fn projects(&self) -> Result<Vec<(String, String)>, String> {
            match &self.err {
                Some(e) => Err(e.clone()),
                None => Ok(self.projects.clone()),
            }
        }
    }

    struct MockWriter {
        ok: bool,
        seen: std::sync::Mutex<Option<serde_json::Value>>,
    }
    #[async_trait::async_trait]
    impl Executor for MockWriter {
        async fn execute(&self, req: &Execute, _grant: &SessionGrant) -> ExecuteOutcome {
            *self.seen.lock().unwrap() = Some(req.tool_input.clone());
            if self.ok {
                ExecuteOutcome::Ok { result: serde_json::json!({}) }
            } else {
                ExecuteOutcome::Error { code: ContractError::ExecutionFailed, message: "refused".into() }
            }
        }
    }

    #[tokio::test]
    async fn run_auto_tag_applies_the_file_part_of_write_for_a_resolved_project() {
        let reader = MockReader { projects: projects(&[("inner", "/home/a/foo")]), err: None };
        let writer = MockWriter { ok: true, seen: std::sync::Mutex::new(None) };
        let r = run_auto_tag("/home/a/foo/x.rs", &reader, &writer).await;
        assert_eq!(r, AutoTagResult::Applied { project: "inner".to_string(), written: true });
        // The daemon-direct write is a FILE_PART_OF from the file (id = path) to
        // the project.
        let input = writer.seen.lock().unwrap().clone().unwrap();
        assert_eq!(input["relation_type"], "FILE_PART_OF");
        assert_eq!(input["from_type"], "File");
        assert_eq!(input["from_id"], "/home/a/foo/x.rs");
        assert_eq!(input["to_type"], "Project");
        assert_eq!(input["to_id"], "inner");
    }

    #[tokio::test]
    async fn run_auto_tag_no_write_on_no_match_ambiguous_or_read_error() {
        let writer = MockWriter { ok: true, seen: std::sync::Mutex::new(None) };
        assert_eq!(
            run_auto_tag("/x.rs", &MockReader { projects: projects(&[("p", "/home/a")]), err: None }, &writer).await,
            AutoTagResult::NoProject
        );
        assert_eq!(
            run_auto_tag("/home/a/x.rs", &MockReader { projects: projects(&[("p1", "/home/a"), ("p2", "/home/a")]), err: None }, &writer).await,
            AutoTagResult::Ambiguous
        );
        assert_eq!(
            run_auto_tag("/home/a/x.rs", &MockReader { projects: vec![], err: Some("boom".into()) }, &writer).await,
            AutoTagResult::ReadError("boom".to_string())
        );
        // No write was attempted in any of these.
        assert!(writer.seen.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn run_auto_tag_reports_not_written_when_the_executor_refuses() {
        // executor_live off -> the write executor refuses -> Applied{written:false}.
        let reader = MockReader { projects: projects(&[("p", "/home/a")]), err: None };
        let writer = MockWriter { ok: false, seen: std::sync::Mutex::new(None) };
        assert_eq!(
            run_auto_tag("/home/a/x.rs", &reader, &writer).await,
            AutoTagResult::Applied { project: "p".to_string(), written: false }
        );
    }
}
