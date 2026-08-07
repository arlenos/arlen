//! `arlen-i18n-lint` - the born-translatable gate (i18n-plan.md I18N-R2).
//!
//! Scans first-party `.svelte` source for hardcoded user-facing strings (text
//! nodes plus a small set of user-facing attributes) and gates on a committed
//! baseline: a string already in the baseline is accepted, a NEW one fails the
//! run. The point is to stop the i18n debt growing while the UI is built;
//! retrofitting the baselined strings into the MF2 catalogs is the later
//! extraction sweep (I18N-R4).
//!
//! The kit's `t()` binding does exist (`@arlen/ui-kit` i18n), so the reason this
//! is a baseline-diff rather than a "route through i18n" check is no longer that
//! there is nothing to route through. It is that most user-facing strings are
//! still literals: a gate demanding every one go through `t()` would fail on
//! the first run and stay failing until the extraction sweep finishes. The
//! baseline holds the line where it is until then.
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
//!     --prune     drop baseline entries this scan can no longer reproduce, and
//!                 ONLY those (then exit 0). Unlike `--update` it never accepts a
//!                 new string, so it is safe to run with new findings outstanding:
//!                 removing an entry can only make the gate stricter. Exists
//!                 because a baseline goes stale as files are rewritten, and the
//!                 obvious approximation - grep the file for the text - disagrees
//!                 with this scanner's own extraction on real entries.
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
            // The camelCase spellings our own components take as props. The
            // hyphenated forms are the HTML attributes; a component that forwards
            // one names it `ariaLabel`, and having only the HTML spelling here
            // made every component's accessible name invisible to the lint.
            | "arialabel"
            | "ariadescription"
            | "ariaplaceholder"
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
    // An entity is markup wearing letters. `&nbsp;` and `&rarr;` are spacing and a
    // glyph, and reporting them put three of them in the baseline where they read
    // as unpaid translation work forever. Strip entities and ask again: `A &amp; B`
    // still has letters and is still copy, `&nbsp;` has none left and is not.
    if !strip_entities(&collapsed).chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    Some(collapsed)
}

/// `text` with HTML character references removed.
///
/// Deliberately loose about what a valid entity is: this only decides whether
/// anything is left over, so over-matching a `&word;` that is not a real entity
/// costs nothing (the surrounding copy still has letters), while under-matching
/// would put markup back in the baseline.
fn strip_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' {
            if let Some(end) = chars[i + 1..]
                .iter()
                .take(12)
                .position(|c| *c == ';')
                .map(|p| i + 1 + p)
            {
                let body = &chars[i + 1..end];
                let entity = !body.is_empty()
                    && body
                        .iter()
                        .all(|c| c.is_ascii_alphanumeric() || *c == '#');
                if entity {
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
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
                        // Expression-valued attribute. Its literals are still
                        // copy when the attribute is one that renders, and the
                        // expression is JavaScript, so the script scanner's own
                        // positions apply inside it too.
                        let vline = line;
                        let body = take_expr(&chars, &mut i, &mut line);
                        if is_displayed_attr(&attr) {
                            for lit in expr_literals(&body) {
                                if let Some(t) = user_facing_text(&lit) {
                                    out.push(Finding {
                                        line: vline,
                                        text: t,
                                    });
                                }
                            }
                        }
                        out.extend(scan_script(&body).into_iter().map(|f| Finding {
                            line: vline + f.line - 1,
                            text: f.text,
                        }));
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
    take_expr(chars, i, line);
}

/// Step over a `{...}` and return what was inside, braces excluded.
///
/// Skipping was right for markup - `{count}` is a value, not copy - but it also
/// discarded `statusText={on ? "Radios off" : "Available"}`, and a conditional
/// prop is how a Svelte component usually picks between two labels. Every tile in
/// the shell's quick settings said its state that way and the gate reported the
/// whole directory clean.
fn take_expr(chars: &[char], i: &mut usize, line: &mut usize) -> String {
    let n = chars.len();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let start = *i;
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
                        let body: String = chars[start + 1..*i].iter().collect();
                        *i += 1;
                        return body;
                    }
                }
            }
        }
        *i += 1;
    }
    chars[start.min(n)..n].iter().collect()
}

/// The property names and call sinks whose string argument reaches the user.
///
/// Scanning every string literal in TypeScript would be a false-positive machine:
/// most literals are keys, ids, class names, MIME types and query fragments. So
/// this does not scan literals, it scans POSITIONS - a literal is only a finding
/// when it sits where the value is displayed. The list is deliberately short and
/// grows when a real miss is found, not speculatively.
const USER_FACING_PROPS: &[&str] = &[
    "label", "title", "placeholder", "description", "ariaLabel", "tooltip", "message",
    "summary", "heading", "hint", "caption", "confirmLabel", "cancelLabel",
    // A tile's state line, which is its whole second row of text. Added on a
    // measured miss: every quick-settings tile said its state through this prop
    // and the gate called the directory clean.
    "statusText",
];

/// Deliberately absent: `name`, `text` and `body`.
///
/// `{ value: "#6366f1", name: "Indigo" }` is a swatch's accessible name and was a
/// genuine miss, so adding `name` looks obviously right. Measured, it produced 129
/// findings against 3 for the rest of this list, and nearly all of them were mock
/// records standing in for a backend: project names, saved-search names, a meeting
/// transcript. Those must NOT be translated - they are the user's data, in a
/// fixture until the daemon behind them lands - so the lint would be wrong 97% of
/// the time on its loudest rule. `name` is the commonest field in any data record,
/// and no shape test separates our word for a thing from the user's name for it.
/// The swatch case is fixed at its source instead.

/// Call heads whose first string argument is shown to the user.
const USER_FACING_CALLS: &[&str] = &[
    "toast", "toast.success", "toast.error", "toast.info", "toast.warning", "alert", "confirm",
];

/// Whether an attribute or component prop puts its value on screen.
///
/// The HTML attributes plus the prop names, because a Svelte component takes
/// `statusText={…}` where an element would take `title="…"` and both end up as
/// text the reader sees.
fn is_displayed_attr(name: &str) -> bool {
    is_user_facing_attr(name)
        || USER_FACING_PROPS
            .iter()
            .any(|p| p.eq_ignore_ascii_case(name))
}

/// Every quoted literal in an expression, unescaped, in order.
///
/// Only the string bodies: what is around them (a ternary, a call, an object)
/// decides nothing here, because the attribute name has already said this
/// position is displayed.
fn expr_literals(src: &str) -> Vec<String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' || c == '\'' {
            i += 1;
            let mut lit = String::new();
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                }
                lit.push(chars[i]);
                i += 1;
            }
            i += 1;
            out.push(lit);
            continue;
        }
        i += 1;
    }
    out
}

/// Whether a string is a fragment of another language rather than copy.
///
/// A helper returning `` `<span class="tok-kw">${w}</span>` `` or a transition
/// returning `opacity: 0; transform: scale(0.98);` is building markup or CSS. It
/// has spaces and letters like a sentence does, so nothing else here separates
/// it, and putting either in the baseline would list syntax as translation work.
///
/// Deliberately four narrow shapes rather than a parser: an opening tag, a
/// custom property, a declaration list, and an attribute selector. Prose does not
/// carry `<div` or `--`, rarely carries a colon and a semicolon at once, and does
/// not carry a bracket pair.
///
/// The selector shape earns its place: `button, a, input, [role='button']` is the
/// focus-trap query, it is returned from a helper in nine components, and it has
/// commas and lowercase words like a list of nouns.
fn is_syntax(t: &str) -> bool {
    let opens_a_tag = t
        .as_bytes()
        .windows(2)
        .any(|w| w[0] == b'<' && (w[1].is_ascii_alphabetic() || w[1] == b'/'));
    opens_a_tag
        || t.contains("--")
        || (t.contains(':') && t.contains(';'))
        || (t.contains('[') && t.contains(']'))
}

/// Whether a literal in a user-facing position reads as prose rather than an
/// identifier. Catalog keys, CSS classes, MIME types and dotted paths all land in
/// these positions legitimately (`label: "chevron-down"` on an icon prop), and
/// flagging them would train people to ignore the lint.
fn user_facing_text(lit: &str) -> Option<String> {
    let t = meaningful(lit)?;
    if is_syntax(&t) {
        return None;
    }
    let has_space = t.contains(' ');
    // An identifier-shaped single token: a dotted key, a path, a MIME type, a
    // kebab or snake name. Prose has spaces or at least starts with a capital.
    if !has_space {
        if t.contains('.') || t.contains('/') || t.contains('_') || t.contains('-') {
            return None;
        }
        if !t.chars().next()?.is_uppercase() {
            return None;
        }
    }
    Some(t)
}

/// Scan TypeScript or JavaScript for hardcoded user-facing strings.
///
/// One forward pass. Line comments and block comments are skipped. At every
/// quoted literal the preceding non-space characters decide whether the position
/// is user-facing: a `prop:` from [`USER_FACING_PROPS`] or a call head from
/// [`USER_FACING_CALLS`].
///
/// Template literals were skipped wholesale on the grounds that prose inside one
/// is a rarer shape. It is not rarer, and it is the worse one: `` `Unpin ${name}` ``
/// is a sentence built by concatenation, which is exactly what cannot be translated
/// into a language that orders those parts differently. Two of them sat in the kit
/// unnoticed because the gate could not see them. They are now reported with each
/// interpolation shown as `{}`, so the finding reads as the shape it is.
fn scan_script(src: &str) -> Vec<Finding> {
    let chars: Vec<char> = src.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    // The last token seen before the current position, used to classify it.
    let mut prefix = String::new();
    // Whether a `return` is still open. The single-token lookbehind sees
    // `return "x"` but not `return cond ? "a" : "b"`, and that ternary is how a
    // helper usually picks between two sentences - `compatLine` in the
    // Windows-apps page is exactly it, and stayed invisible after `return` was
    // added as a position. Cleared at the `;` or `}` that ends the statement,
    // not at a newline: a returned ternary is normally wrapped across lines.
    let mut in_return = false;

    while i < n {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            prefix.clear();
            i += 1;
            continue;
        }
        // Comments.
        if c == '/' && i + 1 < n && chars[i + 1] == '/' {
            while i < n && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && i + 1 < n && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < n && !(chars[i] == '*' && chars[i + 1] == '/') {
                if chars[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        // Template literal: walk it, keeping the static segments and stepping over
        // each `${...}`.
        if c == '`' {
            let start_line = line;
            i += 1;
            // Segments in order, with interpolations rendered as `{}` so the report
            // shows the shape rather than a sentence with a hole in it.
            let mut shape = String::new();
            let mut segments: Vec<String> = Vec::new();
            let mut current = String::new();
            while i < n && chars[i] != '`' {
                if chars[i] == '\\' && i + 1 < n {
                    // An escape contributes its escaped character, not the slash.
                    i += 1;
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    current.push(chars[i]);
                    i += 1;
                    continue;
                }
                if chars[i] == '$' && i + 1 < n && chars[i + 1] == '{' {
                    segments.push(std::mem::take(&mut current));
                    shape.push_str("{}");
                    // Brace depth, so an object literal or a nested template inside
                    // the interpolation does not end it early.
                    let mut depth = 0usize;
                    i += 1;
                    while i < n {
                        match chars[i] {
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    i += 1;
                                    break;
                                }
                            }
                            '\n' => line += 1,
                            _ => {}
                        }
                        i += 1;
                    }
                    continue;
                }
                if chars[i] == '\n' {
                    line += 1;
                }
                current.push(chars[i]);
                shape.push(chars[i]);
                i += 1;
            }
            segments.push(current);
            i += 1;
            // A template with no interpolation is an ordinary string that happens to
            // use backticks, and the whole thing is the text. With interpolations the
            // question is whether any static segment is prose, because that is the
            // concatenation an i18n gate exists to catch: it cannot be reordered for a
            // language that wants the value first.
            // The shape is judged too, not only the segments. `color-mix(in srgb,
            // var(--foreground) ${n}%, transparent)` splits so that the CSS marker
            // lands in the first segment and the second reads as ordinary words,
            // and the finding a human then sees is the whole shape - which is CSS.
            if (in_return || position_is_user_facing(&prefix))
                && !is_syntax(&shape)
                && segments.iter().any(|seg| user_facing_text(seg).is_some())
            {
                if let Some(t) = meaningful(&shape) {
                    out.push(Finding { line: start_line, text: t });
                }
            }
            prefix.clear();
            continue;
        }
        if c == '"' || c == '\'' {
            let quote = c;
            let start_line = line;
            i += 1;
            let mut lit = String::new();
            while i < n && chars[i] != quote {
                if chars[i] == '\\' && i + 1 < n {
                    i += 1;
                }
                if chars[i] == '\n' {
                    line += 1;
                }
                lit.push(chars[i]);
                i += 1;
            }
            i += 1;
            if in_return || position_is_user_facing(&prefix) {
                if let Some(t) = user_facing_text(&lit) {
                    out.push(Finding { line: start_line, text: t });
                }
            }
            prefix.clear();
            continue;
        }
        // Build the preceding-token context. Whitespace before a `(` or `:` is
        // common, so spaces are dropped rather than clearing the prefix.
        if c.is_alphanumeric()
            || c == '_'
            || c == '$'
            || c == '.'
            || c == ':'
            || c == '('
            || c == '='
            || c == '!'
        {
            prefix.push(c);
            if prefix.len() > 64 {
                prefix.drain(..prefix.len() - 64);
            }
        } else {
            if trailing_name(&prefix) == "return" {
                in_return = true;
            }
            // `;` and `}` end the statement; `{` opens an object literal or a
            // block, and inside one the property rules already decide - without
            // this, `return { instance: "All" }` reads every value in the
            // literal as returned copy, and a wire enum lands in the baseline.
            if c == ';' || c == '}' || c == '{' {
                in_return = false;
            }
            if !c.is_whitespace() {
                prefix.clear();
            }
        }
        i += 1;
    }
    out
}

/// Whether the characters immediately before a literal put it in a displayed
/// position: `someProp:` or a known call head's `(`.
fn position_is_user_facing(prefix: &str) -> bool {
    if let Some(head) = prefix.strip_suffix(':') {
        let name = trailing_name(head);
        if USER_FACING_PROPS.iter().any(|p| p.eq_ignore_ascii_case(name)) {
            return true;
        }
    }
    // `prop="text"`: a Svelte component attribute, and a plain assignment to a
    // named variable. This shape was invisible until an `ariaLabel="Interface
    // font"` sat two lines from an `ariaLabel={$t(..)}` and only the second one
    // was a finding. The object-only props are excluded here on purpose.
    if let Some(head) = prefix.strip_suffix('=') {
        // Not a comparison: `a == "x"`, `a !== "x"` and `=>` are all conditions or
        // arrows rather than positions, and the first would otherwise read the
        // operand as a prop name.
        if head.ends_with(['=', '!', '<', '>']) {
            return false;
        }
        let name = trailing_name(head);
        if USER_FACING_PROPS.iter().any(|p| p.eq_ignore_ascii_case(name)) {
            return true;
        }
    }
    if let Some(head) = prefix.strip_suffix('(') {
        return USER_FACING_CALLS.iter().any(|c| head.ends_with(c));
    }
    // `return "sentence"`. A helper that picks a phrase per case is one of the
    // commonest ways prose enters a component, and it was invisible here: the
    // whole `compatLine`/`accessLine` pair in the Windows-apps page sat in the
    // source with the gate reporting no new findings, because a return is not a
    // prop and not a call argument.
    //
    // Measured before adopting, the way `name` was measured before declining:
    // 94 findings, of which 9 were a function returning CSS or a markup fragment
    // and the other 85 were plain English sentences shown to the user. The 9 are
    // filtered by shape in `user_facing_text`, since a CSS declaration is not
    // prose wherever it appears.
    if prefix == "return" {
        return true;
    }
    false
}

/// The identifier at the end of a prefix, which is the prop or variable name.
fn trailing_name(head: &str) -> &str {
    let cut = head.rfind(|c: char| !(c.is_alphanumeric() || c == '_')).map_or(0, |i| i + 1);
    &head[cut..]
}

/// The `<script>` bodies of a Svelte component, so the script scanner sees them.
/// The markup scanner deliberately skips these, which is what left every string
/// built in a component's script invisible to the lint.
fn script_bodies(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let bytes: Vec<char> = src.chars().collect();
    let lower: String = src.to_lowercase();
    let mut search = 0usize;
    while let Some(open) = lower[search..].find("<script") {
        let tag_start = search + open;
        let Some(gt) = lower[tag_start..].find('>') else { break };
        let body_start = tag_start + gt + 1;
        let Some(close) = lower[body_start..].find("</script") else { break };
        let body_end = body_start + close;
        let line = src[..body_start].chars().filter(|c| *c == '\n').count() + 1;
        out.push((line, src[body_start..body_end].to_string()));
        search = body_end;
        let _ = &bytes;
    }
    out
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
        } else if is_scannable(&name) && !is_kit_dev_surface(&path) && !is_guarded_fixture(&path) {
            out.push(path);
        }
    }
}

/// Whether a file is part of the product's surface.
///
/// Tests are excluded, and that is the point rather than a convenience: a test
/// asserts on fixed strings, so `expect(msg).toBe("daemon down")` is a literal in
/// a `message:` position that must never be a catalog entry. Translating it would
/// break the test it belongs to. Sixty of the first hundred findings from the
/// attribute shape were test fixtures, which would have taught people to skim.
fn is_scannable(name: &str) -> bool {
    if !(name.ends_with(".svelte") || name.ends_with(".ts")) {
        return false;
    }
    !(name.ends_with(".test.ts")
        || name.ends_with(".spec.ts")
        || name.ends_with(".test.svelte")
        || name.ends_with(".d.ts"))
}

/// The kit's own dev surfaces, which render for a developer and not for a user.
///
/// Weaker ground than the test-file rule above, and the difference matters. A test
/// file can never be built into an app; that is a property of what it is. These are
/// ordinary routes in a shipped package, skipped only because no app imports them
/// TODAY. That is a fact about the current tree rather than about the files, and it
/// can change without anyone noticing, at which point the exclusion is hiding real
/// user-facing strings.
///
/// So [`kit_dev_surfaces_are_unimported`] checks the premise on every run: if an app
/// ever imports one of these, the exclusion fails loudly instead of quietly
/// covering less.
const KIT_DEV_SURFACES: &[&str] = &[
    "sdk/ui-kit/src/routes/",
    "sdk/ui-kit/src/lib/components/a11y-kitchen.svelte",
];

fn is_kit_dev_surface(path: &Path) -> bool {
    let p = path.to_string_lossy().replace('\\', "/");
    KIT_DEV_SURFACES.iter().any(|s| p.contains(s))
}

/// What an app would have to write to reach one of the skipped surfaces. The kit is
/// consumed as `@arlen/ui-kit/...`, and its routes are not part of that surface at
/// all, so any of these appearing in an app is the premise breaking.
const KIT_DEV_IMPORTS: &[&str] = &["a11y-kitchen", "ui-kit/src/routes", "@arlen/ui-kit/routes"];

/// Fail if an app imports a surface the scan skips.
///
/// Returns the offending (file, marker) pairs. Empty is the healthy state and the
/// only one in which skipping those files is honest.
fn kit_dev_surfaces_are_unimported(files: &[PathBuf]) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for path in files {
        let rel = path.to_string_lossy().replace('\\', "/");
        // Only apps can break the premise. The kit importing its own dev route is
        // what a dev route is for.
        if !rel.contains("apps/") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        for marker in KIT_DEV_IMPORTS {
            if text.contains(marker) {
                out.push((rel.clone(), *marker));
            }
        }
    }
    out
}

/// An app's own screenshot fixture, skipped only while it provably cannot ship.
///
/// `apps/*/src/routes/_*` render mock data for the screenshot harness. They are
/// ordinary routes, so SvelteKit builds them into the app - eleven of them were
/// sitting in the built bundles, reachable by URL - and each now carries a
/// `+page.ts` that 404s unless `dev`.
///
/// That guard is what this reads. A fixture is skipped BECAUSE the tree contains
/// the thing that stops it reaching a user, not because of a convention about the
/// leading underscore. Delete the guard and its strings come back as findings,
/// which is the correct outcome: an unguarded fixture ships, so its strings are
/// user-facing.
fn is_guarded_fixture(path: &Path) -> bool {
    let p = path.to_string_lossy().replace('\\', "/");
    if !p.contains("/routes/_") {
        return false;
    }
    let Some(dir) = path.parent() else { return false };
    let Ok(guard) = std::fs::read_to_string(dir.join("+page.ts")) else { return false };
    guard.contains("$app/environment") && guard.contains("error(404")
}

/// The line ranges of constants declared foreign, and the reason given for each.
///
/// A third of what is left in the baseline is fixture data, and it splits in two.
/// Some of it stands in for data that arrives at runtime from somebody else - a
/// third-party app's setting schema, another app's dbusmenu, a store listing from
/// a server. Those strings are not ours in the fixture for the same reason they
/// are not ours in production: the surface renders supplier text verbatim and
/// cannot translate an arbitrary app's words. The rest stands in for data our own
/// daemons will send, which IS ours, and counting it as paid would hide real debt.
///
/// No regex can tell those apart, because the difference is who owns the data at
/// runtime. So it is declared, above the constant, with a reason:
///
/// ```text
/// // i18n-foreign: a third-party app's labels arrive from the broker as data.
/// const FIXTURE: AppSettingsPage = { ... };
/// ```
///
/// This annotates, it never suppresses. A foreign string still enters the
/// baseline and still counts as new if it appears - only the summary separates
/// it. That is deliberate: a marker that could hide a finding would be reached
/// for whenever a string was awkward, and the gate would quietly stop checking.
/// All this buys is that the headline number means what it says.
///
/// Errs when a marker carries no reason. "i18n-foreign:" alone is the form that
/// gets copied to the next constant without anybody re-deciding.
fn foreign_ranges(src: &str) -> Result<Vec<(usize, usize, String)>, String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        // `export const` too, which is the commoner form: a marker that silently
        // applies to nothing is the same failure as no marker at all, and every
        // list in `themeSystem.ts` is exported.
        //
        // And a function, because a fixture built by one is the same fixture: the
        // knowledge app's timeline assembles its days in `function fixture()`
        // while its three sibling stores assign a `const`, and requiring the
        // author to restructure code so a scanner can see the marker is the wrong
        // way round. The range still ends at the next declaration, so widening
        // what may OPEN a range does not widen how far one reaches.
        let decl = ["const ", "export const ", "function ", "export function "]
            .iter()
            .any(|p| line.starts_with(p));
        if !decl {
            continue;
        }
        // Walk up the contiguous comment block directly above the declaration.
        let mut reason = None;
        let mut j = i;
        while j > 0 {
            let above = lines[j - 1].trim_start();
            if !above.starts_with("//") {
                break;
            }
            if let Some((_, rest)) = above.split_once("i18n-foreign:") {
                reason = Some(rest.trim().to_string());
                break;
            }
            j -= 1;
        }
        let Some(reason) = reason else { continue };
        if reason.is_empty() {
            return Err(format!(
                "line {}: an i18n-foreign marker with no reason. Say whose data it is\n  \
                 and why the surface cannot translate it, or drop the marker.",
                j
            ));
        }
        // Ends at the next thing that starts a declaration or a comment at column
        // zero. Over-running would wrongly mark the next constant's strings, so the
        // rule stops early rather than late: the cost of stopping early is a string
        // counted as debt, which is the safe direction.
        let mut end = lines.len() + 1;
        for (k, l) in lines.iter().enumerate().skip(i + 1) {
            let starts_decl = ["const ", "let ", "var ", "function ", "export ", "class ",
                               "interface ", "type ", "declare ", "//"]
                .iter()
                .any(|p| l.starts_with(p));
            if starts_decl {
                end = k + 1;
                break;
            }
        }
        out.push((i + 1, end, reason));
    }
    Ok(out)
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
    prune: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut roots = Vec::new();
    let mut baseline = PathBuf::from("dev/i18n-baseline.tsv");
    let mut update = false;
    let mut prune = false;
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
            "--prune" => prune = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if roots.is_empty() {
        roots.push(PathBuf::from("apps"));
    }
    Ok(Args { roots, baseline, update, prune })
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

    // The scan skips the kit's dev routes on the premise that no app pulls one in.
    // Check it rather than assume it: an exclusion that quietly stops being true is
    // a gate covering less than its output claims.
    let imported = kit_dev_surfaces_are_unimported(&files);
    if !imported.is_empty() {
        eprintln!(
            "arlen-i18n-lint: an app reaches a ui-kit dev surface, which the scan skips.\n\
             Those files are excluded only because nothing ships them. Either drop the\n\
             import, or remove the path from KIT_DEV_SURFACES so its strings are checked:"
        );
        for (file, marker) in &imported {
            eprintln!("  {file}: {marker}");
        }
        return ExitCode::from(2);
    }

    let mut current: BTreeSet<String> = BTreeSet::new();
    let mut report: Vec<(String, usize, String, bool)> = Vec::new();
    let mut foreign_count = 0usize;
    for path in &files {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("arlen-i18n-lint: cannot read {}: {e}", path.display());
                return ExitCode::from(2);
            }
        };
        let rel = path.to_string_lossy().replace('\\', "/");
        // A component contributes both its markup and its script; a `.ts` module
        // is script only. The script pass is what closes the blind spot that let
        // every option table and toast built in a `<script>` block ship in one
        // language while the markup beside it was fully translated.
        let mut found: Vec<Finding> = Vec::new();
        if rel.ends_with(".svelte") {
            found.extend(scan_svelte(&src));
            for (offset, body) in script_bodies(&src) {
                found.extend(scan_script(&body).into_iter().map(|f| Finding {
                    line: f.line + offset - 1,
                    text: f.text,
                }));
            }
        } else {
            found.extend(scan_script(&src));
        }
        let ranges = match foreign_ranges(&src) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("arlen-i18n-lint: {rel}: {e}");
                return ExitCode::from(2);
            }
        };
        for f in found {
            let k = key(&rel, &f.text);
            if current.insert(k.clone()) {
                let foreign = ranges.iter().any(|(a, b, _)| f.line >= *a && f.line < *b);
                if foreign {
                    foreign_count += 1;
                }
                report.push((rel.clone(), f.line, f.text, foreign));
            }
        }
    }

    if args.prune {
        let baseline: Vec<String> = match std::fs::read_to_string(&args.baseline) {
            Ok(s) => s.lines().map(|l| l.to_string()).collect(),
            Err(e) => {
                eprintln!(
                    "arlen-i18n-lint: cannot read baseline {}: {e}",
                    args.baseline.display()
                );
                return ExitCode::from(2);
            }
        };
        let kept: Vec<&String> = baseline.iter().filter(|k| current.contains(*k)).collect();
        let dropped = baseline.len() - kept.len();
        let body: String = kept.iter().map(|k| format!("{k}\n")).collect();
        if let Err(e) = std::fs::write(&args.baseline, body) {
            eprintln!(
                "arlen-i18n-lint: cannot write baseline {}: {e}",
                args.baseline.display()
            );
            return ExitCode::from(2);
        }
        println!(
            "arlen-i18n-lint: pruned {dropped} unreproducible entr(ies), {} kept -> {}",
            kept.len(),
            args.baseline.display()
        );
        return ExitCode::SUCCESS;
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

    let new: Vec<&(String, usize, String, bool)> = report
        .iter()
        .filter(|(rel, _, text, _)| !baseline.contains(&key(rel, text)))
        .collect();

    if new.is_empty() {
        // The split is the point of the marker: without it the headline counts
        // another party's words as our unpaid work, and a number that overstates
        // what it measures stops being read.
        let ours = current.len() - foreign_count;
        println!(
            "arlen-i18n-lint: ok, {} known user-facing strings, no new ones\n  \
             {ours} ours, {foreign_count} declared foreign data",
            current.len()
        );
        return ExitCode::SUCCESS;
    }

    eprintln!(
        "arlen-i18n-lint: {} NEW hardcoded user-facing string(s) - route through i18n, \
         or run with --update if intentionally non-translatable:",
        new.len()
    );
    for (rel, line, text, foreign) in new {
        // Labelled, not excused: a new string inside a foreign fixture is still a
        // finding, because it might be one of ours put in the wrong place.
        let tag = if *foreign { " [foreign fixture]" } else { "" };
        eprintln!("  {rel}:{line}: {text}{tag}");
    }
    ExitCode::from(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(src: &str) -> Vec<String> {
        scan_svelte(src).into_iter().map(|f| f.text).collect()
    }

    fn script_texts(src: &str) -> Vec<String> {
        scan_script(src).into_iter().map(|f| f.text).collect()
    }

    /// The other half of the same blind spot, and the half the scanner tests
    /// cannot see: a `.ts` module has to be COLLECTED before anything scans it.
    /// The 808 invisible strings were mostly in `src/lib/stores/*.ts`, and if the
    /// `.ts` arm here went away every scanner test above would still pass while
    /// the lint quietly went back to reading markup only.
    #[test]
    fn a_ts_module_is_collected_alongside_components() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let app = root.join("app/src/lib/stores");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("zoom.ts"), "const O = [{ label: \"Continuously\" }];").unwrap();
        std::fs::write(root.join("app/src/Page.svelte"), "<p>Copy</p>").unwrap();
        // Neither of these is ours to lint: one is a dependency, one is not source.
        let deps = root.join("app/node_modules/pkg");
        std::fs::create_dir_all(&deps).unwrap();
        std::fs::write(deps.join("index.ts"), "const O = [{ label: \"Vendor\" }];").unwrap();
        std::fs::write(root.join("app/README.md"), "Documentation copy").unwrap();

        let mut files = Vec::new();
        collect_svelte(root, &mut files);
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(names.contains(&"zoom.ts".to_string()), "collected {names:?}");
        assert!(names.contains(&"Page.svelte".to_string()), "collected {names:?}");
        assert!(!names.contains(&"index.ts".to_string()), "node_modules is not ours");
        assert!(!names.contains(&"README.md".to_string()), "not a source file");
    }

    #[test]
    fn a_label_in_an_option_table_is_found() {
        // The blind spot this pass exists for: the markup scanner skips
        // `<script>`, so a table of option labels shipped untranslated while the
        // markup beside it was fully covered.
        assert_eq!(
            script_texts(r#"const O = [{ label: "Save changes", value: "save" }];"#),
            vec!["Save changes"]
        );
    }

    #[test]
    fn a_toast_argument_is_found() {
        assert_eq!(
            script_texts(r#"toast.error("Could not open the file");"#),
            vec!["Could not open the file"]
        );
    }

    #[test]
    fn identifier_shaped_values_in_the_same_position_are_not_findings() {
        // These land in user-facing positions legitimately - an icon name, a
        // catalog key, a MIME type. Flagging them teaches people to ignore the
        // lint, which is worse than missing one string.
        assert!(script_texts(r#"const a = { label: "chevron-down" };"#).is_empty());
        assert!(script_texts(r#"const b = { label: "s.pr.jbQueued" };"#).is_empty());
        assert!(script_texts(r#"const c = { title: "text/plain" };"#).is_empty());
        assert!(script_texts(r#"const d = { label: "save" };"#).is_empty());
    }

    #[test]
    fn a_literal_outside_a_displayed_position_is_not_a_finding() {
        assert!(script_texts(r#"const cls = "flex items-center";"#).is_empty());
        assert!(script_texts(r#"await invoke("open_file", { path });"#).is_empty());
    }

    #[test]
    fn comments_are_skipped() {
        assert!(script_texts(r#"// label: "A comment is not code""#).is_empty());
        assert!(script_texts("/* label: \"Nor a block comment\" */").is_empty());
        // A template in a position nobody displays stays quiet, so the quoted string
        // inside it is not mistaken for a label.
        assert!(script_texts("const t = `label: \"Not scanned\"`;").is_empty());
    }

    #[test]
    fn an_attribute_and_a_prop_default_are_positions_too() {
        // The shape that was invisible: a Svelte attribute and a `$props()`
        // default both end in `=`, so the prefix was cleared before it could be
        // read. An unoverridden default is what renders, so it counts.
        let hits = scan_script("let { placeholder = \"Add app...\" } = $props();");
        assert_eq!(hits.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), vec!["Add app..."]);
        // Markup goes through the other scanner, which keeps its own attribute
        // list. It had `aria-label` but not the camelCase `ariaLabel` our own
        // components take, so every component's accessible name was invisible.
        let attr = scan_svelte("<X ariaLabel=\"Interface font\" />");
        assert_eq!(attr.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(), vec!["Interface font"]);
    }

    #[test]
    fn a_comparison_is_not_a_position() {
        // `=` also ends every comparison, and reading the left operand as a prop
        // name would make `label == "Files"` a finding about a condition.
        assert!(scan_script("if (label == \"Files\") {}").is_empty());
        assert!(scan_script("if (title !== \"Home page\") {}").is_empty());
    }

    #[test]
    fn an_entity_only_text_node_is_markup_not_copy() {
        assert_eq!(meaningful("&nbsp;"), None);
        assert_eq!(meaningful("&rarr;"), None);
        assert_eq!(meaningful("&#8594;"), None);
        assert_eq!(meaningful(" &nbsp; &nbsp; "), None);
    }

    #[test]
    fn copy_around_an_entity_is_still_copy() {
        // The boundary that matters: stripping entities must not swallow the
        // sentence they sit in.
        assert_eq!(meaningful("Fish &amp; chips"), Some("Fish &amp; chips".into()));
        assert_eq!(meaningful("Save &rarr; Export"), Some("Save &rarr; Export".into()));
    }

    #[test]
    fn a_marker_reaches_an_exported_constant() {
        // Every list in `themeSystem.ts` is exported, and a marker that applied
        // only to a bare `const` sat above them doing nothing at all - which
        // reads exactly like a marker that works.
        let src = "\
// i18n-foreign: installed theme packages name themselves.
export const CURSOR_THEMES = [
  { label: \"Bibata\" },
];
";
        let r = foreign_ranges(src).unwrap();
        assert_eq!(r.len(), 1, "an exported constant was not covered");
    }

    /// A fixture built by a function is the same fixture as one assigned to a
    /// const. This was not accepted, so the marker above `function fixture()`
    /// applied to nothing - the failure the `export const` case was already
    /// widened to avoid.
    #[test]
    fn a_marker_opens_a_range_on_a_function_too() {
        let src = "// i18n-foreign: the user's own projects.\n\
function fixture() {\n\
  return [{ project: \"Website redesign\" }];\n\
}\n\
const OURS = [\"Settings\"];\n";
        let r = foreign_ranges(src).unwrap();
        assert_eq!(r.len(), 1, "{r:?}");
        let (start, end, reason) = &r[0];
        assert_eq!(reason, "the user's own projects.");
        // Opens at the function, closes before the next declaration, so the
        // string that IS ours stays outside it.
        assert_eq!((*start, *end), (2, 5), "{r:?}");
    }

    #[test]
    fn a_declared_foreign_constant_covers_its_own_lines_and_no_more() {
        let src = "\
// i18n-foreign: a third-party app's labels arrive as data.
const FIXTURE = [
  { label: \"Save automatically\" },
];

const OURS = [
  { label: \"Show sidebar\" },
];
";
        let r = foreign_ranges(src).unwrap();
        assert_eq!(r.len(), 1);
        let (start, end, reason) = &r[0];
        assert!(reason.contains("third-party"));
        // The fixture's label is inside; the constant after it is not. Over-running
        // here would launder our own strings as somebody else's.
        assert!((*start..*end).contains(&3), "fixture label at line 3 not covered");
        assert!(!(*start..*end).contains(&8), "the next constant was swallowed");
    }

    #[test]
    fn an_unmarked_constant_is_ours() {
        let src = "const OURS = [\n  { label: \"Show sidebar\" },\n];\n";
        assert!(foreign_ranges(src).unwrap().is_empty());
    }

    #[test]
    fn a_marker_without_a_reason_is_refused() {
        // The form that gets copied to the next constant without anybody deciding
        // again. If it passed, the marker would spread and the count would drift
        // back to meaning nothing.
        let src = "// i18n-foreign:\nconst FIXTURE = [\n  { label: \"x\" },\n];\n";
        assert!(foreign_ranges(src).is_err());
    }

    #[test]
    fn the_marker_must_sit_on_the_constant_it_describes() {
        // A marker separated by code belongs to nothing, and honouring it at a
        // distance is how one justification ends up covering a whole file.
        let src = "\
// i18n-foreign: the store's own listings.
const FIRST = [];
const SECOND = [
  { label: \"Save\" },
];
";
        let r = foreign_ranges(src).unwrap();
        assert_eq!(r.len(), 1);
        assert!(!(r[0].0..r[0].1).contains(&4), "the marker reached past its constant");
    }

    #[test]
    fn a_fixture_route_is_skipped_only_while_it_is_guarded() {
        let root = std::env::temp_dir().join(format!("arlen-i18n-fx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let guarded = root.join("apps/x/src/routes/_shown");
        let bare = root.join("apps/x/src/routes/_ships");
        std::fs::create_dir_all(&guarded).unwrap();
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::write(guarded.join("+page.svelte"), "<p>Fixture copy</p>").unwrap();
        std::fs::write(
            guarded.join("+page.ts"),
            "import { dev } from \"$app/environment\";\nexport const load = () => { if (!dev) error(404); };",
        )
        .unwrap();
        // Same shape, no guard: this one is built into the app, so its strings
        // are a user's problem and must still be scanned.
        std::fs::write(bare.join("+page.svelte"), "<p>Fixture copy</p>").unwrap();

        assert!(is_guarded_fixture(&guarded.join("+page.svelte")));
        assert!(!is_guarded_fixture(&bare.join("+page.svelte")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_test_file_is_not_a_surface() {
        // A test asserts on fixed strings; translating one breaks the test.
        assert!(!is_scannable("export.test.ts"));
        assert!(!is_scannable("engine.spec.ts"));
        assert!(!is_scannable("types.d.ts"));
        assert!(is_scannable("store.ts"));
        assert!(is_scannable("Page.svelte"));
    }

    #[test]
    fn prose_in_a_template_literal_is_reported_with_its_shape() {
        // The shape this exists for: a sentence assembled by concatenation, which
        // cannot be reordered for a language that wants the value elsewhere.
        assert_eq!(script_texts("{ label: `Unpin ${place.label}` }"), vec!["Unpin {}"]);
        assert_eq!(
            script_texts("{ tooltip: `${p.label} (not connected)` }"),
            vec!["{} (not connected)"],
        );
        // Braces inside the interpolation must not end it early.
        assert_eq!(
            script_texts("{ title: `Saved ${fmt({ n: 2 })} files` }"),
            vec!["Saved {} files"],
        );
    }

    #[test]
    fn a_template_without_prose_or_without_a_display_position_is_quiet() {
        // Not a displayed position.
        assert!(script_texts("const cls = `chip-${kind}`;").is_empty());
        // Displayed, but every static segment is identifier-shaped rather than prose:
        // a class name or a unit suffix glued to a value is not a sentence.
        assert!(script_texts("{ label: `${n}-${m}` }").is_empty());
    }

    #[test]
    fn a_script_body_is_located_with_its_line_offset() {
        let src = "<div>x</div>\n<script>\n  const o = { label: \"Save changes\" };\n</script>";
        let bodies = script_bodies(src);
        assert_eq!(bodies.len(), 1);
        let (offset, body) = &bodies[0];
        let f = &scan_script(body)[0];
        // The finding must point at the real file line, not the body-relative one.
        assert_eq!(f.line + offset - 1, 3);
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

    #[test]
    fn a_conditional_accessible_name_is_a_finding() {
        // The shape that hid every one of them: two names picked by a ternary
        // inside the attribute expression, which the markup scanner skipped
        // wholesale because `{...}` is usually a value rather than copy.
        assert_eq!(
            texts(r#"<button aria-label={muted ? "Unmute" : "Mute"}>x</button>"#),
            vec!["Unmute", "Mute"]
        );
        assert_eq!(
            texts(r#"<Tile statusText={on ? "Radios off" : "Available"} />"#),
            vec!["Radios off", "Available"]
        );
    }

    #[test]
    fn an_expression_in_a_structural_attribute_is_not() {
        // `class` and the like carry the same shape and none of the meaning.
        assert!(texts(r#"<div class={big ? "card-lg" : "card-sm"}>x</div>"#).is_empty());
        assert!(texts(r#"<div style={wide ? "width: 40rem" : "width: 20rem"}>x</div>"#).is_empty());
    }

    #[test]
    fn a_key_comparison_inside_a_handler_is_not() {
        // `onkeydown` is not a displayed attribute, and the literal is a
        // comparison rather than a label, so neither path should claim it.
        assert!(texts(r#"<input onkeydown={(e) => e.key === "Enter" && go()} />"#).is_empty());
    }

    #[test]
    fn a_call_inside_an_expression_is_still_scanned() {
        // The expression is JavaScript, so the script scanner's positions hold
        // inside it too.
        assert_eq!(
            texts(r#"<button onclick={() => toast("Saved to Documents")}>x</button>"#),
            vec!["Saved to Documents"]
        );
    }

    #[test]
    fn a_returned_sentence_is_a_finding() {
        // The shape that hid `compatLine` and `accessLine` in the Windows-apps
        // page: prose picked per case by a helper, returned rather than assigned.
        let src = r#"
            function accessLine(b) {
              if (!b.network) return "Limited access. It cannot reach your network.";
              return "Broad access. It can reach your network.";
            }"#;
        assert_eq!(
            script_texts(src),
            vec![
                "Limited access. It cannot reach your network.",
                "Broad access. It can reach your network.",
            ]
        );
    }

    #[test]
    fn a_returned_identifier_is_not() {
        // The reason `return` could not simply be added: helpers return keys,
        // classes and paths far more often than sentences, and the existing shape
        // test is what keeps those out.
        assert!(script_texts(r#"return "chevron-down";"#).is_empty());
        assert!(script_texts(r#"return "text/plain";"#).is_empty());
        assert!(script_texts(r#"return "s.wa.windowed";"#).is_empty());
    }

    #[test]
    fn a_returned_css_or_markup_fragment_is_not() {
        // Both have letters and spaces, so only their syntax separates them from
        // copy. Listing either in the baseline would read as translation work.
        assert!(script_texts(r#"return "opacity: 0; transform: scale(0.98);";"#).is_empty());
        assert!(script_texts(r#"return `<span class="tok-kw">${w}</span>`;"#).is_empty());
        assert!(
            script_texts(r#"return `color-mix(in srgb, var(--fg) ${p}%, transparent)`;"#)
                .is_empty()
        );
    }

    #[test]
    fn a_returned_ternary_reports_both_branches() {
        // `compatLine`: the branch is the sentence, so the lookbehind on its own
        // sees a `?` and a `:` and nothing else.
        let src = r#"
            function compatLine(b) {
              return b.tier === "curated"
                ? `Curated and verified, using the ${b.recipe}`
                : "Best effort on the default setup, it may not run perfectly";
            }"#;
        assert_eq!(
            script_texts(src),
            vec![
                "Curated and verified, using the {}",
                "Best effort on the default setup, it may not run perfectly",
            ]
        );
    }

    #[test]
    fn a_returned_selector_is_not_copy() {
        let src = r#"return "button, a, input, [role='button']";"#;
        assert!(script_texts(src).is_empty());
    }

    #[test]
    fn a_returned_object_literal_is_judged_by_its_props() {
        // `instance` is a wire enum, not copy, and only the prop rules can know
        // that. Without the brace ending the return, every value in a returned
        // literal read as a returned sentence.
        assert!(script_texts(r#"return { instance: "All", mode: "Own" };"#).is_empty());
        assert_eq!(
            script_texts(r#"return { instance: "All", label: "Everything here" };"#),
            vec!["Everything here"]
        );
    }

    #[test]
    fn a_return_does_not_leak_past_its_statement() {
        // The flag has to close, or every literal after the first return in a
        // file is judged as though it were being returned.
        let src = r#"
            function f() { return "A sentence here."; }
            const icon = { name: "Another sentence here." };"#;
        assert_eq!(script_texts(src), vec!["A sentence here."]);
    }

    #[test]
    fn a_sentence_with_a_colon_is_still_copy() {
        // The declaration-list test needs both marks, because prose uses a colon
        // on its own: "Active task: {}" is a sentence, not CSS.
        assert_eq!(
            script_texts(r#"return `Active task: ${name}.`;"#),
            vec!["Active task: {}."]
        );
    }
}
