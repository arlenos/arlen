//! The Obsidian vault markdown "floor" reader (foreign-app-bridges.md, the
//! Obsidian lead case).
//!
//! This is the no-plugin tier: it reads a note file's frontmatter, `#tags` and
//! `[[wikilinks]]` straight from the `.md` text into the flat inbound message
//! the [`crate::interpret`] mapping consumes, so a vault can be ingested with no
//! Obsidian plugin installed. (The richer plugin tier supplies Obsidian's own
//! resolved link graph + live events; this floor parses the file itself.)
//!
//! The parse is pure and deliberately a pragmatic subset, documented per field:
//! frontmatter is the common flat `key: value`, inline `[a, b]` and block `- `
//! list forms (not arbitrary nested YAML - a vault's frontmatter is overwhelmingly
//! flat); `#tags` and `[[wikilinks]]` are scanned from the body with fenced and
//! inline code masked out so a code sample never injects a tag or a link. The
//! file-watch that feeds this and the wiring into the interpreter are separate
//! slices; this is the parser they share.

use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::path::Path;

/// Parse a note's text into the flat message body the bridge interpreter
/// consumes: every frontmatter key verbatim, plus a `tags` array (the
/// frontmatter `tags` merged with the body `#tags`) and a `links` array (the
/// body `[[wikilink]]` targets), both deduplicated and sorted. Pure.
///
/// `tags`/`links` always present (possibly empty) so a `bridge.toml` rule can
/// project them unconditionally. A frontmatter `tags` key is consumed into the
/// merged tag set rather than left as a raw string, so the message carries one
/// canonical tag list.
pub fn parse_note(content: &str) -> Map<String, Value> {
    let (frontmatter, body) = split_frontmatter(content);

    let mut out = Map::new();
    let mut tags: BTreeSet<String> = BTreeSet::new();

    for (key, value) in frontmatter {
        if key == "tags" {
            // Fold the frontmatter tags into the merged set instead of emitting
            // a raw `tags` field (the body tags join them below).
            match value {
                Value::Array(items) => {
                    for item in items {
                        if let Some(t) = item.as_str() {
                            collect_tag(t, &mut tags);
                        }
                    }
                }
                Value::String(s) => {
                    // A flat `tags: a, b` or single `tags: a` line.
                    for part in s.split(',') {
                        collect_tag(part.trim(), &mut tags);
                    }
                }
                _ => {}
            }
            continue;
        }
        out.insert(key, value);
    }

    let masked = mask_code(body);
    for tag in scan_tags(&masked) {
        tags.insert(tag);
    }
    let links: BTreeSet<String> = scan_wikilinks(&masked).into_iter().collect();

    out.insert(
        "tags".to_string(),
        Value::Array(tags.into_iter().map(Value::String).collect()),
    );
    out.insert(
        "links".to_string(),
        Value::Array(links.into_iter().map(Value::String).collect()),
    );
    out
}

/// Assemble the full inbound `note` message for a vault file: the parsed
/// content ([`parse_note`]) plus the two fields the file supplies rather than
/// the text - `path` (the vault-relative path, the stable idempotency key a
/// `bridge.toml` rule keys on) and `title` (the frontmatter `title` if it is a
/// non-empty string, else the file's stem). `rel_path` is the path relative to
/// the vault root, using `/` separators. Pure.
pub fn note_message(rel_path: &str, content: &str) -> Map<String, Value> {
    let mut msg = parse_note(content);
    let title = msg
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| stem_of(rel_path));
    msg.insert("path".to_string(), Value::String(rel_path.to_string()));
    msg.insert("title".to_string(), Value::String(title));
    msg
}

/// The file stem of a `/`-separated relative path: the last component with a
/// trailing `.md` (or any extension) removed. The display fallback for a note
/// with no frontmatter `title`.
fn stem_of(rel_path: &str) -> String {
    let name = rel_path.rsplit('/').next().unwrap_or(rel_path);
    match name.rsplit_once('.') {
        Some((stem, _ext)) if !stem.is_empty() => stem.to_string(),
        _ => name.to_string(),
    }
}

/// Read a vault's markdown floor: walk `root` recursively for `.md` files and
/// assemble each into its [`note_message`], keyed by its vault-relative path
/// (`/`-separated). This is the one-shot initial sync (the live file-watch that
/// re-emits on change is a separate slice); an unreadable individual file is
/// skipped best-effort so one bad note never aborts the sync. Results are sorted
/// by path for determinism. Returns an error only if `root` itself is unreadable.
pub fn scan_vault(root: &Path) -> std::io::Result<Vec<Map<String, Value>>> {
    let mut files: Vec<(String, String)> = Vec::new();
    walk_markdown(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files
        .into_iter()
        .map(|(rel, content)| note_message(&rel, &content))
        .collect())
}

/// The vault-relative, `/`-separated path of `path` if it is a note the floor
/// ingests: under `root`, a `.md` file (case-insensitive leaf), with no hidden
/// path component (a name starting with `.`, e.g. anything under `.obsidian`).
/// `None` otherwise. Mirrors [`scan_vault`]'s filter so the live file-watch and
/// the one-shot sync agree on what counts as a note.
pub fn vault_relative_md(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut parts: Vec<String> = Vec::new();
    for component in rel.components() {
        match component {
            std::path::Component::Normal(name) => {
                let name = name.to_string_lossy();
                if name.starts_with('.') {
                    return None;
                }
                parts.push(name.into_owned());
            }
            // `..`, a root or prefix component is never a plain vault note path.
            _ => return None,
        }
    }
    let leaf = parts.last()?;
    if !leaf.to_ascii_lowercase().ends_with(".md") {
        return None;
    }
    Some(parts.join("/"))
}

/// Recursively collect `(vault-relative path, content)` for every `.md` file
/// under `dir`. A hidden entry (a name starting with `.`, e.g. Obsidian's
/// `.obsidian` config dir) is skipped; a file that cannot be read is skipped.
fn walk_markdown(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if file_type.is_dir() {
            // A subdirectory's read failure is skipped, not fatal.
            let _ = walk_markdown(root, &path, out);
        } else if file_type.is_file() && name.to_ascii_lowercase().ends_with(".md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(rel) = path.strip_prefix(root) {
                    let rel = rel.to_string_lossy().replace('\\', "/");
                    out.push((rel, content));
                }
            }
        }
    }
    Ok(())
}

/// Add a tag string to the set, normalised: a leading `#` stripped, surrounding
/// whitespace trimmed, empty or purely-numeric rejected (Obsidian's rule: a tag
/// must contain a non-digit, so `#2024` is not a tag).
fn collect_tag(raw: &str, tags: &mut BTreeSet<String>) {
    let t = raw.trim().trim_start_matches('#').trim();
    if t.is_empty() || t.chars().all(|c| c.is_ascii_digit()) {
        return;
    }
    tags.insert(t.to_string());
}

/// Split a leading `---` YAML frontmatter block from the body. Returns the
/// parsed frontmatter pairs and the remaining body. With no frontmatter the
/// pairs are empty and the body is the whole input.
fn split_frontmatter(content: &str) -> (Vec<(String, Value)>, &str) {
    // Frontmatter must be the very first line (allowing a leading BOM/newline is
    // not Obsidian's contract: the `---` opens column 0, line 1).
    let rest = match content.strip_prefix("---\n") {
        Some(r) => r,
        None => return (Vec::new(), content),
    };
    // The block ends at a line that is exactly `---` (or `...`, YAML's other
    // end marker). Find it.
    let mut idx = 0usize;
    let mut block_end: Option<(usize, usize)> = None; // (block_len, body_start)
    while idx < rest.len() {
        let line_end = rest[idx..].find('\n').map(|n| idx + n);
        let line = match line_end {
            Some(e) => &rest[idx..e],
            None => &rest[idx..],
        };
        if line == "---" || line == "..." {
            let body_start = line_end.map(|e| e + 1).unwrap_or(rest.len());
            block_end = Some((idx, body_start));
            break;
        }
        match line_end {
            Some(e) => idx = e + 1,
            None => break,
        }
    }
    match block_end {
        Some((block_len, body_start)) => {
            let block = &rest[..block_len];
            (parse_frontmatter_block(block), &rest[body_start..])
        }
        // An unterminated frontmatter block: treat the whole input as body (no
        // frontmatter), fail-safe rather than swallow the note.
        None => (Vec::new(), content),
    }
}

/// Parse the frontmatter block's pragmatic YAML subset into ordered pairs:
/// `key: value` scalars, `key: [a, b]` inline lists, and a `key:` header
/// followed by indented `- item` lines (block lists). Unrecognised lines are
/// skipped rather than guessed.
fn parse_frontmatter_block(block: &str) -> Vec<(String, Value)> {
    let mut pairs: Vec<(String, Value)> = Vec::new();
    let mut lines = block.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim().is_empty() {
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let rest = rest.trim();
        if rest.is_empty() {
            // A block list: collect the following `- item` lines.
            let mut items: Vec<Value> = Vec::new();
            while let Some(peek) = lines.peek() {
                let t = peek.trim_start();
                if let Some(item) = t.strip_prefix("- ") {
                    items.push(Value::String(unquote(item.trim()).to_string()));
                    lines.next();
                } else if t.starts_with('-') && t.len() == 1 {
                    lines.next();
                } else {
                    break;
                }
            }
            pairs.push((key.to_string(), Value::Array(items)));
        } else if let Some(inner) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            // An inline list `[a, b, c]`.
            let items = inner
                .split(',')
                .map(|p| Value::String(unquote(p.trim()).to_string()))
                .filter(|v| v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
                .collect();
            pairs.push((key.to_string(), Value::Array(items)));
        } else {
            pairs.push((key.to_string(), Value::String(unquote(rest).to_string())));
        }
    }
    pairs
}

/// Strip a single pair of surrounding single or double quotes, if present.
fn unquote(s: &str) -> &str {
    let b = s.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

/// Replace fenced (``` / ~~~) and inline (`` ` ``) code regions with spaces so a
/// code sample's `#x` or `[[x]]` is never scanned as a tag or a link, while line
/// and column offsets stay stable. Returns the masked body.
fn mask_code(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.extend(line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
            continue;
        }
        if in_fence {
            out.extend(line.chars().map(|c| if c == '\n' { '\n' } else { ' ' }));
            continue;
        }
        // Mask inline `code` spans within the line.
        let mut in_inline = false;
        for c in line.chars() {
            if c == '`' {
                in_inline = !in_inline;
                out.push(' ');
            } else if in_inline && c != '\n' {
                out.push(' ');
            } else {
                out.push(c);
            }
        }
    }
    out
}

/// Scan body `#tags` (Obsidian rule: a `#` not preceded by a word character,
/// followed by tag characters `[A-Za-z0-9_/-]`, the tag containing at least one
/// non-digit). Nested tags (`#area/sub`) are kept whole.
fn scan_tags(masked: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let chars: Vec<char> = masked.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '#' {
            let prev_is_word = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
            if !prev_is_word {
                let mut j = i + 1;
                while j < chars.len() && is_tag_char(chars[j]) {
                    j += 1;
                }
                if j > i + 1 {
                    let tag: String = chars[i + 1..j].iter().collect();
                    if !tag.chars().all(|c| c.is_ascii_digit()) {
                        tags.push(tag);
                    }
                }
                i = j;
                continue;
            }
        }
        i += 1;
    }
    tags
}

/// A character permitted inside an Obsidian tag body.
fn is_tag_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '/'
}

/// Scan `[[wikilink]]` targets, resolving `[[target|alias]]`,
/// `[[target#heading]]` and `[[target^block]]` to the bare target. Empty
/// targets are dropped.
fn scan_wikilinks(masked: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = masked;
    while let Some(open) = rest.find("[[") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("]]") else {
            break;
        };
        let inner = &after[..close];
        // The target is everything before the first alias/heading/block marker.
        let target = inner
            .split(['|', '#', '^'])
            .next()
            .unwrap_or("")
            .trim();
        if !target.is_empty() {
            links.push(target.to_string());
        }
        rest = &after[close + 2..];
    }
    links
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(m: &Map<String, Value>) -> Vec<String> {
        m["tags"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
    }
    fn links(m: &Map<String, Value>) -> Vec<String> {
        m["links"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn frontmatter_scalar_inline_list_and_block_list_parse() {
        let note = "---\ntitle: My Note\naliases: [Alt, Other]\nstatus:\n  - draft\n  - review\n---\nbody\n";
        let m = parse_note(note);
        assert_eq!(m["title"], Value::String("My Note".into()));
        assert_eq!(m["aliases"], serde_json::json!(["Alt", "Other"]));
        assert_eq!(m["status"], serde_json::json!(["draft", "review"]));
    }

    #[test]
    fn body_tags_merge_with_frontmatter_tags_deduped_and_sorted() {
        let note = "---\ntags: [project, work]\n---\nSee #work and #area/sub here.\n";
        let m = parse_note(note);
        // frontmatter {project, work} + body {work, area/sub} -> sorted unique.
        assert_eq!(tags(&m), vec!["area/sub", "project", "work"]);
        // The raw frontmatter `tags` field is consumed into the merged set.
        assert!(m.get("tags").unwrap().is_array());
    }

    #[test]
    fn a_purely_numeric_hash_is_not_a_tag() {
        let m = parse_note("Issue #2024 and #v2 here.\n");
        assert_eq!(tags(&m), vec!["v2"], "#2024 is numeric-only, not a tag");
    }

    #[test]
    fn a_hash_after_a_word_is_not_a_tag() {
        // An anchor like `page#section` or a colour `#fff` mid-word is excluded
        // when preceded by a word char.
        let m = parse_note("url path#frag should not tag, but #real does.\n");
        assert_eq!(tags(&m), vec!["real"]);
    }

    #[test]
    fn wikilinks_resolve_alias_heading_and_block_to_the_target() {
        let m = parse_note("See [[Target]], [[Other|alias]], [[Note#Heading]] and [[Ref^block]].\n");
        assert_eq!(links(&m), vec!["Note", "Other", "Ref", "Target"]);
    }

    #[test]
    fn fenced_and_inline_code_do_not_yield_tags_or_links() {
        let note = "Real #tag and [[Real]].\n```\n#notatag and [[NotALink]]\n```\nInline `#nope [[NoLink]]` too.\n";
        let m = parse_note(note);
        assert_eq!(tags(&m), vec!["tag"]);
        assert_eq!(links(&m), vec!["Real"]);
    }

    #[test]
    fn a_note_without_frontmatter_parses_body_only() {
        let m = parse_note("Just #a body with [[Link]].\n");
        assert_eq!(tags(&m), vec!["a"]);
        assert_eq!(links(&m), vec!["Link"]);
        assert!(m.get("title").is_none());
    }

    #[test]
    fn an_unterminated_frontmatter_block_is_treated_as_body() {
        // No closing `---`: fail-safe to body-only rather than swallow the note.
        let m = parse_note("---\ntitle: x\nbody continues with #tag\n");
        assert!(m.get("title").is_none(), "the open block is not parsed as frontmatter");
        assert_eq!(tags(&m), vec!["tag"]);
    }

    #[test]
    fn quotes_are_stripped_from_scalar_values() {
        let m = parse_note("---\ntitle: \"Quoted Title\"\nslug: 'my-slug'\n---\n");
        assert_eq!(m["title"], Value::String("Quoted Title".into()));
        assert_eq!(m["slug"], Value::String("my-slug".into()));
    }

    #[test]
    fn note_message_injects_path_and_derives_title_from_the_stem() {
        let m = note_message("notes/Ideas.md", "no frontmatter, just #thoughts.\n");
        assert_eq!(m["path"], Value::String("notes/Ideas.md".into()));
        // No frontmatter title -> the filename stem.
        assert_eq!(m["title"], Value::String("Ideas".into()));
        assert_eq!(tags(&m), vec!["thoughts"]);
    }

    #[test]
    fn note_message_prefers_a_frontmatter_title() {
        let m = note_message("notes/x.md", "---\ntitle: Real Title\n---\nbody\n");
        assert_eq!(m["title"], Value::String("Real Title".into()));
        assert_eq!(m["path"], Value::String("notes/x.md".into()));
    }

    #[test]
    fn the_shipped_floor_bridge_maps_a_note_to_a_keyed_upsert_with_no_edges() {
        // The reference floor mapping must parse and interpret an assembled note
        // message into the expected `md.obsidian.Note` upsert, end-to-end, so the
        // committed bridge.toml cannot drift from the floor reader's message shape.
        use crate::bridge::BridgeConfig;
        use crate::interpret::interpret_message;

        let config = BridgeConfig::parse(include_str!("../examples/obsidian/bridge.toml"))
            .expect("the shipped floor bridge.toml parses and validates");

        let msg = note_message(
            "notes/Ideas.md",
            "---\ntitle: Bright Ideas\ntags: [project]\n---\nSee [[Other Note]] and #work.\n",
        );
        let plan = interpret_message(&config, "note", &msg).expect("the note interprets");

        assert_eq!(plan.qualified_type, "md.obsidian.Note");
        // Keyed by the vault-relative path (the idempotency anchor).
        assert_eq!(plan.external_key, "notes/Ideas.md");
        assert_eq!(plan.fields["title"], Value::String("Bright Ideas".into()));
        assert_eq!(plan.fields["tags"], serde_json::json!(["project", "work"]));
        assert_eq!(plan.fields["links"], serde_json::json!(["Other Note"]));
        // The floor emits NO resolved edges (that is the plugin tier's job).
        assert!(plan.links.is_empty(), "the floor mapping declares no for_each_link");
    }

    #[test]
    fn scan_vault_walks_md_files_skips_hidden_and_keys_by_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Top.md"), "# Top with [[Other]]\n").unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("Nested.md"), "---\ntitle: Nested Note\n---\n#tag\n").unwrap();
        std::fs::write(root.join("notes.txt"), "not markdown").unwrap();
        std::fs::create_dir(root.join(".obsidian")).unwrap();
        std::fs::write(root.join(".obsidian").join("config.md"), "#hiddenshouldskip\n").unwrap();

        let msgs = scan_vault(root).unwrap();
        let paths: Vec<&str> = msgs.iter().map(|m| m["path"].as_str().unwrap()).collect();
        // Sorted, `.md` only, hidden `.obsidian` skipped, non-md ignored.
        assert_eq!(paths, vec!["Top.md", "sub/Nested.md"]);
        // The nested note carries its frontmatter title + tag.
        let nested = &msgs[1];
        assert_eq!(nested["title"], Value::String("Nested Note".into()));
        assert_eq!(tags(nested), vec!["tag"]);
        // The top note's wikilink is captured.
        assert_eq!(links(&msgs[0]), vec!["Other"]);
    }
}
