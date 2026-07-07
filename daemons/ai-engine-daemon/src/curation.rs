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
}
