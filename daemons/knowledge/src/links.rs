//! Deterministic cross-content link extraction (kg-richness-plan.md Thrust 3c:
//! "wiki/markdown links, document citations - cheap, exact, no noise"). The pure
//! core: given a markdown/text document's content and the directory it lives in,
//! resolve its outbound `[text](path)` and `[[wikilink]]` references to the
//! absolute file paths they point at (the `File` node id form), so a background
//! pass can write `LINKS_TO` edges between documents.
//!
//! Pure and content-in: it takes the already-read content as a string, performs
//! no file I/O, and only ever returns resolved PATHS (never content), so it adds
//! no hard-exclude surface of its own. The promotion trigger
//! (`promotion::link_markdown_document`) reads a promoted markdown document's
//! content, extracts links, and persists `LINKS_TO` edges to already-observed
//! Files; a private/incognito session is excluded upstream before promotion.
//! `allow(dead_code)` is kept because this module also compiles in the lib tree,
//! which has no promotion consumer.
#![allow(dead_code)]

use crate::graph::GraphHandle;
use crate::utils::escape_cypher;
use anyhow::Result;
use std::collections::BTreeSet;

/// URL schemes (and the anchor marker) whose targets are NOT local files, so
/// they are never resolved to a `LINKS_TO` edge.
const EXTERNAL_PREFIXES: &[&str] = &[
    "http://", "https://", "ftp://", "mailto:", "tel:", "data:", "//",
];

/// Extract the local-file link targets a document references, resolved to
/// absolute paths against `base_dir` (the directory the document lives in).
///
/// Handles markdown inline links `[label](target)` (an optional `"title"` after
/// the target is dropped) and wiki links `[[target]]` / `[[target|label]]`. A
/// `#fragment` suffix is stripped (a link into a section is a link to the file).
/// External URLs, anchor-only links (`#section`) and empty targets are skipped.
/// A target starting with `/` is treated as already-absolute; a relative target
/// is joined to `base_dir` and normalised (`.`/`..` resolved syntactically, no
/// filesystem access). Output is sorted and de-duplicated so a document linking
/// the same file twice yields one edge and the result is deterministic.
pub fn extract_markdown_links(content: &str, base_dir: &str) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for raw in raw_targets(content) {
        if let Some(path) = resolve_target(&raw, base_dir) {
            out.insert(path);
        }
    }
    out.into_iter().collect()
}

/// Persist a document's outbound links as `LINKS_TO` edges (KG-richness Thrust
/// 3c). Takes the source document's File path and the already-resolved target
/// paths (from [`extract_markdown_links`]); creates a `LINKS_TO` edge from the
/// source File to each target File that ALREADY EXISTS in the graph (an edge is
/// never created to an unobserved file, so no speculative/dangling File node is
/// minted). A self-link is skipped. Idempotent (MERGE), so re-promoting the same
/// document adds no duplicates. Returns the number of edges that now exist for
/// the given targets. The source File is assumed already present (it is the
/// document being promoted); a missing source simply links nothing.
pub async fn persist_document_links(
    graph: &GraphHandle,
    source_path: &str,
    targets: &[String],
) -> Result<usize> {
    let src = escape_cypher(source_path);
    let mut linked = 0;
    for target in targets {
        if target == source_path {
            continue;
        }
        let dst = escape_cypher(target);
        // Both endpoints must already exist; MATCH yields no rows for an
        // unobserved target, so MERGE runs only when the target File is real.
        graph
            .write(format!(
                "MATCH (s:File {{id: '{src}'}}), (t:File {{id: '{dst}'}})
                 MERGE (s)-[:LINKS_TO]->(t)"
            ))
            .await?;
        // Confirm the edge exists (it does iff the target File existed).
        let rs = graph
            .query_rows(format!(
                "MATCH (:File {{id: '{src}'}})-[:LINKS_TO]->(:File {{id: '{dst}'}}) \
                 RETURN count(*) AS c"
            ))
            .await?;
        if rs.rows.first().and_then(|r| r.first()).map(|v| v.as_i64()).unwrap_or(0) > 0 {
            linked += 1;
        }
    }
    Ok(linked)
}

/// The raw, unresolved target strings, in document order (deduped + resolved by
/// the caller). Scans for markdown inline-link `](...)` and wiki `[[...]]` forms.
fn raw_targets(content: &str) -> Vec<String> {
    let bytes = content.as_bytes();
    let mut targets = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Wiki link: [[ target (| label)? ]]
        if bytes[i] == b'[' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            if let Some(end) = find(content, "]]", i + 2) {
                let inner = &content[i + 2..end];
                let target = inner.split('|').next().unwrap_or(inner).trim();
                if !target.is_empty() {
                    targets.push(target.to_string());
                }
                i = end + 2;
                continue;
            }
        }
        // Markdown inline link: ](  target (optional "title") )
        if bytes[i] == b']' && i + 1 < bytes.len() && bytes[i + 1] == b'(' {
            if let Some(end) = find(content, ")", i + 2) {
                let inner = &content[i + 2..end];
                // Drop an optional title: target "Some title" or target 'x'.
                let target = inner
                    .split_once(['"', '\''])
                    .map(|(t, _)| t)
                    .unwrap_or(inner)
                    .trim();
                if !target.is_empty() {
                    targets.push(target.to_string());
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    targets
}

/// Byte index of `needle` in `hay` at or after `from`, or `None`.
fn find(hay: &str, needle: &str, from: usize) -> Option<usize> {
    hay.get(from..).and_then(|s| s.find(needle)).map(|p| p + from)
}

/// Resolve one raw target to an absolute local-file path, or `None` if it is not
/// a local file reference (external URL, anchor-only, or empty after stripping).
fn resolve_target(raw: &str, base_dir: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    if EXTERNAL_PREFIXES.iter().any(|p| lower.starts_with(p)) {
        return None;
    }
    // Strip a #fragment / ?query (a link into a section still targets the file).
    let path = raw
        .split(['#', '?'])
        .next()
        .unwrap_or(raw)
        .trim();
    if path.is_empty() {
        // An anchor-only link (#section) targets the document itself: no edge.
        return None;
    }
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{}/{}", base_dir.trim_end_matches('/'), path)
    };
    Some(normalize_path(&joined))
}

/// Syntactically normalise an absolute path: resolve `.` and `..` over the path
/// components without touching the filesystem. A leading `/` is preserved; `..`
/// at the root is dropped (cannot escape above root).
fn normalize_path(path: &str) -> String {
    let mut stack: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            p => stack.push(p),
        }
    }
    format!("/{}", stack.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_markdown_link() {
        let links = extract_markdown_links("see [the notes](sub/b.md) here", "/home/tim/proj");
        assert_eq!(links, vec!["/home/tim/proj/sub/b.md"]);
    }

    #[test]
    fn resolves_wiki_link_and_strips_label() {
        let links = extract_markdown_links("ref [[notes/c|My Notes]] end", "/home/tim/proj");
        assert_eq!(links, vec!["/home/tim/proj/notes/c"]);
    }

    #[test]
    fn parent_relative_is_normalised() {
        let links = extract_markdown_links("[x](../shared/d.md)", "/home/tim/proj/docs");
        assert_eq!(links, vec!["/home/tim/proj/shared/d.md"]);
    }

    #[test]
    fn absolute_target_is_kept() {
        let links = extract_markdown_links("[x](/etc/notes/e.md)", "/home/tim/proj");
        assert_eq!(links, vec!["/etc/notes/e.md"]);
    }

    #[test]
    fn external_urls_and_anchors_are_skipped() {
        let c = "[a](https://example.com) [b](mailto:x@y.z) [c](#section) [d](//cdn/x)";
        assert!(extract_markdown_links(c, "/p").is_empty());
    }

    #[test]
    fn fragment_suffix_is_stripped_to_the_file() {
        let links = extract_markdown_links("[x](guide.md#install)", "/home/tim/proj");
        assert_eq!(links, vec!["/home/tim/proj/guide.md"]);
    }

    #[test]
    fn title_after_target_is_dropped() {
        let links = extract_markdown_links("[x](f.md \"A title\")", "/p");
        assert_eq!(links, vec!["/p/f.md"]);
    }

    #[test]
    fn duplicate_links_collapse_and_sort() {
        let c = "[a](z.md) [b](a.md) [c](z.md)";
        assert_eq!(extract_markdown_links(c, "/p"), vec!["/p/a.md", "/p/z.md"]);
    }

    #[test]
    fn no_links_is_empty() {
        assert!(extract_markdown_links("plain text, no links", "/p").is_empty());
        assert!(extract_markdown_links("", "/p").is_empty());
    }

    #[test]
    fn unterminated_forms_do_not_panic_or_match() {
        assert!(extract_markdown_links("[[unterminated and ](also", "/p").is_empty());
    }

    #[test]
    fn surplus_parent_refs_are_absorbed_not_escaped() {
        // The resolved path becomes a File-node lookup key, so a traversal-heavy
        // target must normalise to a real absolute path with no surviving `..`,
        // never a relative escape sequence, however many `../` it stacks.
        let links = extract_markdown_links("[x](../../../../etc/x.md)", "/home/u/p");
        assert_eq!(links, vec!["/etc/x.md"]);
        assert!(!links[0].contains(".."), "no `..` survives normalisation");
    }

    #[test]
    fn query_suffix_is_stripped_to_the_file() {
        // A `?query` suffix targets the same file as the bare path, like `#`.
        let links = extract_markdown_links("[x](page.md?v=2)", "/p");
        assert_eq!(links, vec!["/p/page.md"]);
    }

    #[test]
    fn external_scheme_match_is_case_insensitive() {
        // A scheme in any case is still an external URL, never a local file.
        let c = "[a](HTTPS://example.com/x) [b](MailTo:x@y.z)";
        assert!(extract_markdown_links(c, "/p").is_empty());
    }

    /// LINKS_TO edges are created only to files that already exist, never to an
    /// unobserved target, are idempotent, and skip a self-link.
    #[tokio::test]
    async fn persist_links_only_to_existing_files_idempotently() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("g").to_str().unwrap()).unwrap();
        for id in ["/d/a.md", "/d/b.md", "/d/c.md"] {
            graph.write(format!("MERGE (f:File {{id: '{id}'}})")).await.unwrap();
        }
        // a links to b (exists), c (exists), z (does NOT exist), and itself.
        let targets = vec![
            "/d/b.md".to_string(),
            "/d/c.md".to_string(),
            "/d/z.md".to_string(),
            "/d/a.md".to_string(),
        ];
        let n = persist_document_links(&graph, "/d/a.md", &targets).await.unwrap();
        assert_eq!(n, 2, "only the two existing non-self targets link");

        let count = graph
            .query_rows("MATCH (:File {id:'/d/a.md'})-[:LINKS_TO]->() RETURN count(*) AS c".into())
            .await
            .unwrap();
        assert_eq!(count.rows[0][0].as_i64(), 2, "two edges, no self/dangling link");

        // No File node was minted for the unobserved target.
        let z = graph
            .query_rows("MATCH (f:File {id:'/d/z.md'}) RETURN count(*) AS c".into())
            .await
            .unwrap();
        assert_eq!(z.rows[0][0].as_i64(), 0, "an unobserved target is not created");

        // Re-running is idempotent (MERGE): still two edges.
        let n2 = persist_document_links(&graph, "/d/a.md", &targets).await.unwrap();
        assert_eq!(n2, 2);
        let again = graph
            .query_rows("MATCH (:File {id:'/d/a.md'})-[:LINKS_TO]->() RETURN count(*) AS c".into())
            .await
            .unwrap();
        assert_eq!(again.rows[0][0].as_i64(), 2, "re-persist does not duplicate edges");
    }
}
