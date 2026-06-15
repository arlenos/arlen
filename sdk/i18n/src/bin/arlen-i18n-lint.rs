//! `arlen-i18n-lint` - the born-translatable gate (i18n-plan.md I18N-R2).
//!
//! Scans first-party `.svelte` source for hardcoded user-facing strings (text
//! nodes plus a small set of user-facing attributes) and gates on a committed
//! baseline: a string already in the baseline is accepted, a NEW one fails the
//! run. The point is to stop the i18n debt growing while the UI is built;
//! retrofitting the baselined strings into the MF2 catalogs is the later
//! extraction sweep (I18N-R4), and a frontend `t()` binding does not exist yet,
//! so a baseline-diff (not a "route through i18n" check) is the only honest gate
//! today.
//!
//! The detector is heuristic by design (no full Svelte parse): it skips
//! `<script>`/`<style>` blocks, HTML comments and `{...}` expressions, then flags
//! letter-bearing text runs and the literal values of user-facing attributes. It
//! is a debt-growth gate, not a translation oracle - conservative, deterministic,
//! and tuned so the baseline is real UI copy rather than punctuation or glyph
//! noise. False entries can be pruned from the baseline by hand; the gate only
//! ever cares about strings that are NOT in the baseline.
//!
//! Usage:
//!   arlen-i18n-lint [--root <dir>]... [--baseline <file>] [--update]
//!     --root      a directory tree to scan (repeatable; default `apps`)
//!     --baseline  the accepted-strings file (default `dev/i18n-baseline.tsv`)
//!     --update    rewrite the baseline from the current findings (then exit 0)
//! Exit code 0 = no new strings (or `--update`); 1 = new strings; 2 = usage/IO.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// A hardcoded user-facing string found in a source file. The baseline key is
/// `(relative-path, text)`; `line` is reporting-only, so a string that merely
/// moves lines is not mistaken for a new finding.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Finding {
    line: usize,
    text: String,
}

/// The attributes whose literal values are user-facing copy. Deliberately small:
/// only attributes that render as visible or assistive text. `value`/`href`/
/// `class`/`id`/`role`/`data-*` and the like are excluded (not user copy, or
/// usually dynamic).
fn is_user_facing_attr(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "placeholder"
            | "title"
            | "alt"
            | "label"
            | "aria-label"
            | "aria-description"
            | "aria-placeholder"
            | "aria-valuetext"
    )
}

/// Collapse a candidate string to its comparison form: trim, collapse internal
/// whitespace runs to a single space. Returns `None` when the result is not
/// meaningful user copy - fewer than two characters, or carrying no letter (pure
/// punctuation, numbers, separators, icon glyphs).
fn meaningful(raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() < 2 {
        return None;
    }
    if !collapsed.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    Some(collapsed)
}

/// Whether `chars[i..]` begins with `pat` (ASCII, case-insensitive).
fn starts_with_ci(chars: &[char], i: usize, pat: &str) -> bool {
    let pat: Vec<char> = pat.chars().collect();
    if i + pat.len() > chars.len() {
        return false;
    }
    chars[i..i + pat.len()]
        .iter()
        .zip(&pat)
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Scan one Svelte source for hardcoded user-facing strings. A single forward
/// pass over the characters: text content is accumulated and flushed at every
/// tag/expression/comment boundary, `<script>`/`<style>` bodies and `{...}`
/// expressions and `<!-- -->` comments are skipped, and inside an open tag the
/// user-facing attributes' quoted literal values are checked. Line numbers are
/// 1-based and track the start of each finding.
fn scan_svelte(src: &str) -> Vec<Finding> {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut text = String::new();
    let mut text_line = 1usize;
    let mut i = 0usize;

    // Flush the accumulated text run as a finding if it is meaningful copy.
    macro_rules! flush_text {
        () => {{
            if let Some(t) = meaningful(&text) {
                out.push(Finding { line: text_line, text: t });
            }
            text.clear();
        }};
    }

    while i < n {
        let c = chars[i];

        // HTML comment: <!-- ... -->
        if c == '<' && starts_with_ci(&chars, i, "<!--") {
            flush_text!();
            i += 4;
            while i < n && !starts_with_ci(&chars, i, "-->") {
                if chars[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 3; // consume "-->"
            continue;
        }

        // Tag: <name ...> or </name> or <name .../>
        if c == '<' {
            flush_text!();
            i += 1;
            // closing slash
            if i < n && chars[i] == '/' {
                i += 1;
            }
            // tag name
            let mut name = String::new();
            while i < n && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_') {
                name.push(chars[i]);
                i += 1;
            }
            let raw_tag = name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style");

            // Scan the tag body (attributes) until the matching '>'.
            let mut attr = String::new();
            while i < n && chars[i] != '>' {
                let ch = chars[i];
                if ch == '\n' {
                    line += 1;
                }
                if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == ':' {
                    attr.push(ch);
                    i += 1;
                    continue;
                }
                if ch == '=' {
                    // value follows
                    i += 1;
                    while i < n && (chars[i] == ' ' || chars[i] == '\t') {
                        i += 1;
                    }
                    if i < n && (chars[i] == '"' || chars[i] == '\'') {
                        let quote = chars[i];
                        i += 1;
                        let vline = line;
                        let mut value = String::new();
                        let mut has_expr = false;
                        while i < n && chars[i] != quote {
                            if chars[i] == '\n' {
                                line += 1;
                            }
                            if chars[i] == '{' {
                                has_expr = true;
                            }
                            value.push(chars[i]);
                            i += 1;
                        }
                        i += 1; // closing quote
                        if !has_expr && is_user_facing_attr(&attr) {
                            if let Some(t) = meaningful(&value) {
                                out.push(Finding { line: vline, text: t });
                            }
                        }
                    } else if i < n && chars[i] == '{' {
                        // expression-valued attribute: skip the {...}
                        skip_expr(&chars, &mut i, &mut line);
                    }
                    attr.clear();
                    continue;
                }
                // any other char (whitespace, '/', quote not after '=') ends the attr name
                attr.clear();
                i += 1;
            }
            if i < n {
                i += 1; // consume '>'
            }

            if raw_tag {
                // Skip the raw body until the matching close tag.
                let close = format!("</{}", name.to_ascii_lowercase());
                while i < n && !starts_with_ci(&chars, i, &close) {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                // consume the close tag up to '>'
                while i < n && chars[i] != '>' {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                if i < n {
                    i += 1;
                }
            }
            continue;
        }

        // Svelte expression / block: { ... } (incl. {#if}, {@html}, {expr})
        if c == '{' {
            flush_text!();
            skip_expr(&chars, &mut i, &mut line);
            continue;
        }

        if c == '\n' {
            line += 1;
        }
        if text.is_empty() {
            text_line = line;
        }
        text.push(c);
        i += 1;
    }
    flush_text!();
    out
}

/// Skip a `{...}` region starting at `*i` (which must point at `{`), advancing
/// `*i` past the matching `}` and counting newlines into `*line`. Brace depth is
/// tracked, and quotes (`'`, `"`, backtick) inside the expression are honored so
/// a brace inside a string literal does not close the region early.
fn skip_expr(chars: &[char], i: &mut usize, line: &mut usize) {
    let n = chars.len();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    while *i < n {
        let ch = chars[*i];
        if ch == '\n' {
            *line += 1;
        }
        match quote {
            Some(q) => {
                if ch == '\\' {
                    *i += 2;
                    continue;
                }
                if ch == q {
                    quote = None;
                }
            }
            None => {
                if ch == '\'' || ch == '"' || ch == '`' {
                    quote = Some(ch);
                } else if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                    if depth == 0 {
                        *i += 1;
                        return;
                    }
                }
            }
        }
        *i += 1;
    }
}

/// Recursively collect `.svelte` files under `root`, skipping vendored and build
/// trees. Returned paths are whatever `root` yields (callers pass repo-relative
/// roots so the keys stay stable across machines). The list is sorted for a
/// deterministic baseline.
fn collect_svelte(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if matches!(
                name.as_ref(),
                "node_modules" | "build" | ".svelte-kit" | "target" | ".git" | "dist"
            ) {
                continue;
            }
            collect_svelte(&path, out);
        } else if name.ends_with(".svelte") {
            out.push(path);
        }
    }
}

/// The baseline key for a finding: `relative/path.svelte\ttext`. Line is excluded
/// so a string that moves within a file is not a new finding.
fn key(rel: &str, text: &str) -> String {
    format!("{rel}\t{text}")
}

struct Args {
    roots: Vec<PathBuf>,
    baseline: PathBuf,
    update: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut roots = Vec::new();
    let mut baseline = PathBuf::from("dev/i18n-baseline.tsv");
    let mut update = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--root" => roots.push(PathBuf::from(
                it.next().ok_or("--root needs a value")?,
            )),
            "--baseline" => {
                baseline = PathBuf::from(it.next().ok_or("--baseline needs a value")?)
            }
            "--update" => update = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::from("apps"));
    }
    Ok(Args { roots, baseline, update })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("arlen-i18n-lint: {e}");
            return ExitCode::from(2);
        }
    };

    // Collect findings as baseline keys, keeping one example line per key for the
    // human-readable report.
    let mut files = Vec::new();
    for root in &args.roots {
        collect_svelte(root, &mut files);
    }
    files.sort();

    let mut current: BTreeSet<String> = BTreeSet::new();
    let mut report: Vec<(String, usize, String)> = Vec::new();
    for path in &files {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("arlen-i18n-lint: cannot read {}: {e}", path.display());
                return ExitCode::from(2);
            }
        };
        let rel = path.to_string_lossy().replace('\\', "/");
        for f in scan_svelte(&src) {
            let k = key(&rel, &f.text);
            if current.insert(k.clone()) {
                report.push((rel.clone(), f.line, f.text));
            }
        }
    }

    if args.update {
        let body: String = current
            .iter()
            .map(|k| format!("{k}\n"))
            .collect::<String>();
        if let Some(parent) = args.baseline.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&args.baseline, body) {
            eprintln!(
                "arlen-i18n-lint: cannot write baseline {}: {e}",
                args.baseline.display()
            );
            return ExitCode::from(2);
        }
        println!(
            "arlen-i18n-lint: baseline updated with {} strings -> {}",
            current.len(),
            args.baseline.display()
        );
        return ExitCode::SUCCESS;
    }

    let baseline: BTreeSet<String> = match std::fs::read_to_string(&args.baseline) {
        Ok(s) => s.lines().map(|l| l.to_string()).collect(),
        Err(_) => BTreeSet::new(), // missing baseline => everything is new
    };

    let new: Vec<&(String, usize, String)> = report
        .iter()
        .filter(|(rel, _, text)| !baseline.contains(&key(rel, text)))
        .collect();

    if new.is_empty() {
        println!(
            "arlen-i18n-lint: ok, {} known user-facing strings, no new ones",
            current.len()
        );
        return ExitCode::SUCCESS;
    }

    eprintln!(
        "arlen-i18n-lint: {} NEW hardcoded user-facing string(s) - route through i18n, \
         or run with --update if intentionally non-translatable:",
        new.len()
    );
    for (rel, line, text) in new {
        eprintln!("  {rel}:{line}: {text}");
    }
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str) -> Vec<String> {
        scan_svelte(src).into_iter().map(|f| f.text).collect()
    }

    #[test]
    fn flags_a_plain_text_node() {
        assert_eq!(texts("<span>Hello world</span>"), vec!["Hello world"]);
    }

    #[test]
    fn collapses_whitespace_and_tracks_line() {
        let f = scan_svelte("<div>\n   Save\n   changes\n</div>");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].text, "Save changes");
        assert_eq!(f[0].line, 2); // the run starts on line 2
    }

    #[test]
    fn ignores_script_and_style_bodies() {
        let src = "<script>const label = 'Hello';</script>\
                   <style>.x { content: 'Bye'; }</style>\
                   <p>Visible</p>";
        assert_eq!(texts(src), vec!["Visible"]);
    }

    #[test]
    fn ignores_expressions_but_keeps_surrounding_text() {
        // The dynamic part is an expression; the literal "items" is real copy.
        assert_eq!(texts("<span>{count} items</span>"), vec!["items"]);
    }

    #[test]
    fn ignores_block_directives() {
        let src = "{#if open}<span>Open</span>{:else}<span>Closed</span>{/if}";
        assert_eq!(texts(src), vec!["Open", "Closed"]);
    }

    #[test]
    fn flags_user_facing_attributes_only() {
        let src = r#"<input placeholder="Search files" class="big" type="text" />"#;
        assert_eq!(texts(src), vec!["Search files"]);
    }

    #[test]
    fn ignores_expression_valued_attributes() {
        let src = r#"<button title={tooltip} aria-label="Close window">x</button>"#;
        // title is dynamic (skipped), aria-label is literal copy, "x" is too short.
        assert_eq!(texts(src), vec!["Close window"]);
    }

    #[test]
    fn ignores_punctuation_numbers_and_single_chars() {
        let src = "<span>:</span><span>42</span><span>x</span><span>1.5%</span>";
        assert!(texts(src).is_empty());
    }

    #[test]
    fn ignores_html_comments() {
        assert_eq!(texts("<!-- Translators: hi -->\n<p>Real</p>"), vec!["Real"]);
    }

    #[test]
    fn handles_brace_inside_expression_string() {
        // The `}` inside the string must not close the expression early; "after"
        // is the only literal text.
        assert_eq!(texts("<span>{ a ? '}' : '{' } after</span>"), vec!["after"]);
    }

    #[test]
    fn does_not_flag_class_or_data_attrs() {
        let src = r#"<div class="card big" data-id="home" id="main">Content here</div>"#;
        assert_eq!(texts(src), vec!["Content here"]);
    }
}
