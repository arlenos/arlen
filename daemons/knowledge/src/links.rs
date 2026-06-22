//! Deterministic cross-content link extraction (kg-richness-plan.md Thrust 3c:
//! "wiki/markdown links, document citations - cheap, exact, no noise"). The pure
//! core: given a markdown/text document's content and the directory it lives in,
//! resolve its outbound `[text](path)` and `[[wikilink]]` references to the
//! absolute file paths they point at (the `File` node id form), so a background
//! pass can write `LINKS_TO` edges between documents.
//!
//! Pure and content-in: it takes the already-read content as a string, performs
//! no file I/O, and only ever returns resolved PATHS (never content), so it adds
//! no hard-exclude surface of its own. The eventual promotion trigger that reads
//! a document's content must honour the hard-exclude invariant (never read a
//! secret/credential file); that wiring is the deferred step. Lives behind
//! `allow(dead_code)` until then (mechanism before trigger).
#![allow(dead_code)]

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
}
