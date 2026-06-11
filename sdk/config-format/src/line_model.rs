//! One shared line-oriented model backing the four hand-rolled formats: INI /
//! `.conf`, Firefox `prefs.js`, `.env`, and flat `key=value`. They differ only
//! in a few axes (section-awareness, the key/value separator, the comment
//! introducer, the value-quoting rule, an optional statement wrapper), so a
//! single tested engine parameterised by [`LineDialect`] replaces four
//! copy-pasted parsers.
//!
//! The model is a `Vec<Line>` over the original text. Every line keeps its raw
//! bytes, so a `set`/`remove` rewrites only the matched key's value run (or
//! deletes its whole line) and re-emits every other line verbatim. That is what
//! makes the round-trip lossless: comments, blank lines, ordering and the exact
//! whitespace of untouched lines are the original `raw` strings, never
//! re-serialized.
//!
//! Each handler ([`crate::handlers::ini`], [`crate::handlers::env`],
//! [`crate::handlers::flat`], [`crate::handlers::firefox`]) is a thin wrapper:
//! it picks a [`LineDialect`], builds a [`LineModel`], and exposes
//! `read`/`set`/`remove` over it.

use crate::error::ParseError;
use crate::model::{ConfigModel, ConfigValue};

/// The largest line-model input accepted, mirroring [`crate::MAX_CONFIG_BYTES`].
/// Enforced before parsing so a hostile multi-megabyte file is refused, not
/// walked.
const MAX_LINE_MODEL_BYTES: usize = crate::MAX_CONFIG_BYTES;

/// How a dialect quotes a value when re-serializing it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum QuoteStyle {
    /// No quoting: the value is written as a bare token. Used by INI and flat
    /// `key=value`, where the line is everything after the separator.
    Bare,
    /// Shell-style: quote with double quotes only when the value needs it
    /// (whitespace, the comment char, a quote, an `=`), escaping `"`/`\`. Used by
    /// `.env`.
    Shell,
    /// JavaScript string/literal rules: a string value becomes a double-quoted
    /// JS string (escaping `"`, `\`, and control chars); a bool/int is a bare
    /// literal. Used by Firefox `prefs.js` inside the `user_pref(...)` wrapper.
    JsLiteral,
}

/// The per-format configuration of the shared line engine.
#[derive(Debug, Clone)]
pub struct LineDialect {
    /// Whether `[section]` headers scope keys (INI). When false, every key lives
    /// in the implicit root section and a key-path is just the key.
    pub sections: bool,
    /// The characters that separate a key from its value, in preference order.
    /// The first one found on a line splits it; a rewrite re-uses the line's
    /// own separator run verbatim, so this only governs parsing and new-line
    /// insertion. INI/flat accept `=` and `:`; `.env` accepts `=`.
    pub separators: &'static [char],
    /// The separator written when INSERTING a brand-new key (must be one of
    /// `separators`). Existing keys keep their own separator.
    pub insert_separator: &'static str,
    /// The character that introduces a whole-line comment when it is the first
    /// non-whitespace character. `#` for INI/.env/flat, `//` is handled by the
    /// Firefox dialect specially (see [`LineDialect::statement`]).
    pub comment_char: char,
    /// How a value is quoted on rewrite/insert.
    pub quote: QuoteStyle,
    /// When set, only lines matching this statement shape carry a key. Firefox
    /// uses `user_pref` so that `pref(...)`, comments and blank lines are all
    /// unmodelled. `None` means every `key<sep>value` line is a key (INI/.env/
    /// flat).
    pub statement: Option<StatementShape>,
}

/// The `user_pref("key", value);` statement shape for Firefox `prefs.js`. A line
/// is a modelled pref only when it matches `<fn>(\s*"<key>"\s*,\s*<value>\s*);`.
#[derive(Debug, Clone, Copy)]
pub struct StatementShape {
    /// The function name, e.g. `user_pref`.
    pub func: &'static str,
}

/// One parsed line of the document. Only [`Line::KeyVal`] carries a modelled
/// key; everything else is preserved verbatim.
#[derive(Debug, Clone)]
enum Line {
    /// A `[section]` header (INI). `raw` is the verbatim line; `name` is the
    /// parsed section name.
    Section {
        /// The verbatim line text (no trailing newline).
        raw: String,
        /// The parsed section name.
        name: String,
    },
    /// A modelled `key<sep>value` (or `user_pref(...)`) line.
    KeyVal(KeyVal),
    /// Any other line: a comment, a blank line, or content the dialect does not
    /// model (a `pref(...)` for Firefox). Preserved verbatim.
    Other {
        /// The verbatim line text (no trailing newline).
        raw: String,
    },
}

/// A parsed key/value line, decomposed so a rewrite touches only the value run.
#[derive(Debug, Clone)]
struct KeyVal {
    /// The verbatim line, kept so an unmodified line re-emits exactly.
    raw: String,
    /// The parsed key (the section-scoped local key, not the dotted path).
    key: String,
    /// The parsed scalar value.
    value: ConfigValue,
    /// The byte range, within `raw`, of the value token to replace on a `set`.
    /// For a quoted value this is the run INCLUDING the quotes, so re-quoting
    /// replaces them cleanly.
    value_span: std::ops::Range<usize>,
}

/// The full parsed document plus the dialect it was parsed with and whether it
/// ended without a trailing newline (so re-emission reproduces that exactly).
pub struct LineModel {
    dialect: LineDialect,
    lines: Vec<Line>,
    /// Whether the original text ended with a newline. A file that does not end
    /// in `\n` must re-emit without one, and an insert must add a leading `\n`
    /// before the new line so it does not glue onto the last line.
    trailing_newline: bool,
}

impl LineModel {
    /// Parse `text` under `dialect` into a line model. Total and panic-free;
    /// oversize input is the only hard failure (a structurally odd line just
    /// becomes [`Line::Other`], so the model never rejects valid-but-unusual
    /// content).
    pub fn parse(text: &str, dialect: LineDialect) -> Result<LineModel, ParseError> {
        if text.len() > MAX_LINE_MODEL_BYTES {
            return Err(ParseError::TooLarge);
        }
        let trailing_newline = text.ends_with('\n');
        let mut lines = Vec::new();
        let mut current_section: Option<String> = None;

        for raw_line in split_lines(text) {
            let raw = raw_line.to_string();
            let trimmed = raw.trim_start();

            // A blank line or a whole-line comment is preserved as-is. Firefox's
            // `//` comments are caught here too (comment_char handles `#`, and
            // the explicit `//` check handles JS line comments).
            if trimmed.is_empty()
                || trimmed.starts_with(dialect.comment_char)
                || trimmed.starts_with("//")
            {
                lines.push(Line::Other { raw });
                continue;
            }

            // A section header, only when the dialect uses sections.
            if dialect.sections {
                if let Some(name) = parse_section_header(trimmed) {
                    current_section = Some(name.clone());
                    lines.push(Line::Section { raw, name });
                    continue;
                }
            }

            // A modelled key line, per the dialect's statement shape.
            if let Some(kv) = parse_key_line(&raw, &dialect, current_section.as_deref()) {
                lines.push(Line::KeyVal(kv));
            } else {
                lines.push(Line::Other { raw });
            }
        }

        Ok(LineModel {
            dialect,
            lines,
            trailing_newline,
        })
    }

    /// Collect the modelled (key-path, value) pairs in document order.
    pub fn to_model(&self) -> ConfigModel {
        let mut entries = Vec::new();
        for line in &self.lines {
            if let Line::KeyVal(kv) = line {
                entries.push((kv.key.clone(), kv.value.clone()));
            }
        }
        ConfigModel::from_entries(entries)
    }

    /// Produce the text resulting from setting `key` to `value`, preserving every
    /// other line verbatim. If `key` is absent it is inserted minimally: appended
    /// to the end of its section (INI), or to the end of the document (the
    /// section-less dialects), with a leading newline only when needed so it
    /// never glues onto a previous line.
    pub fn set(&self, key: &str, value: &ConfigValue) -> Result<String, String> {
        // Locate an existing modelled key.
        if let Some(idx) = self.find_key_index(key) {
            let Line::KeyVal(kv) = &self.lines[idx] else {
                return Err("internal: key index did not point at a key line".to_string());
            };
            let new_token = self.serialize_value(value)?;
            let mut new_raw = kv.raw.clone();
            new_raw.replace_range(kv.value_span.clone(), &new_token);
            let mut rebuilt = self.clone_lines();
            rebuilt[idx] = Line::Other { raw: new_raw };
            return Ok(self.emit(&rebuilt, false));
        }
        // Insert a new key.
        self.insert_key(key, value)
    }

    /// Produce the text resulting from removing `key`: its whole line is dropped
    /// (and only that line). An absent key is a no-op that returns the document
    /// unchanged. A trailing comment that lives ON the key's line goes with it
    /// (it is the key's own decor); a comment on its own preceding line is a
    /// separate [`Line::Other`] and is kept.
    pub fn remove(&self, key: &str) -> Result<String, String> {
        let Some(idx) = self.find_key_index(key) else {
            return Ok(self.emit(&self.clone_lines(), false));
        };
        let mut rebuilt = self.clone_lines();
        rebuilt.remove(idx);
        Ok(self.emit(&rebuilt, false))
    }

    /// The index of the line carrying the modelled key-path `key`, if any.
    fn find_key_index(&self, key: &str) -> Option<usize> {
        self.lines.iter().position(|line| match line {
            Line::KeyVal(kv) => kv.key == key,
            _ => false,
        })
    }

    /// Clone the line vector so an edit can rebuild without mutating `self`.
    fn clone_lines(&self) -> Vec<Line> {
        self.lines.clone()
    }

    /// Insert a brand-new `key = value` line minimally. For a sectioned dialect
    /// the key-path splits into `section.local`; the line is appended after the
    /// last line of the matching section (creating the section at document end if
    /// absent). For a section-less dialect the line is appended at the end.
    fn insert_key(&self, key: &str, value: &ConfigValue) -> Result<String, String> {
        let token = self.serialize_value(value)?;
        let mut rebuilt = self.clone_lines();

        if self.dialect.sections {
            let (section, local) = split_section_key(key)
                .ok_or_else(|| format!("key-path {key:?} has no section"))?;
            let new_line = self.format_key_line(local, &token);
            match self.last_line_of_section(&rebuilt, section) {
                Some(insert_at) => rebuilt.insert(insert_at + 1, Line::Other { raw: new_line }),
                None => {
                    // Create the section at the document end, then the key.
                    rebuilt.push(Line::Other {
                        raw: format!("[{section}]"),
                    });
                    rebuilt.push(Line::Other { raw: new_line });
                }
            }
        } else {
            let new_line = self.format_key_line(key, &token);
            rebuilt.push(Line::Other { raw: new_line });
        }

        Ok(self.emit(&rebuilt, false))
    }

    /// The index of the last line that belongs to `section` (the section header
    /// itself, or the last key/comment line before the next header). `None` if
    /// the section does not exist.
    fn last_line_of_section(&self, lines: &[Line], section: &str) -> Option<usize> {
        let header = lines.iter().position(|l| match l {
            Line::Section { name, .. } => name == section,
            _ => false,
        })?;
        // Walk to the line before the next section header (or end).
        let mut last = header;
        for (i, line) in lines.iter().enumerate().skip(header + 1) {
            if matches!(line, Line::Section { .. }) {
                break;
            }
            last = i;
        }
        Some(last)
    }

    /// Format a new `key<sep>value` (or `user_pref(...)`) line for insertion.
    fn format_key_line(&self, key: &str, token: &str) -> String {
        match &self.dialect.statement {
            Some(stmt) => format!("{}(\"{}\", {});", stmt.func, key, token),
            None => format!("{}{}{}", key, self.dialect.insert_separator, token),
        }
    }

    /// Serialize a [`ConfigValue`] to its on-line token under the dialect's
    /// quoting rule. [`ConfigValue::Opaque`] is unsettable.
    fn serialize_value(&self, value: &ConfigValue) -> Result<String, String> {
        if matches!(value, ConfigValue::Opaque) {
            return Err("cannot serialize an opaque (non-scalar) value".to_string());
        }
        Ok(match self.dialect.quote {
            QuoteStyle::Bare => match value {
                ConfigValue::String(s) => s.clone(),
                ConfigValue::Bool(b) => b.to_string(),
                ConfigValue::Int(i) => i.to_string(),
                ConfigValue::Float(f) => format_float(*f),
                ConfigValue::Opaque => unreachable!("guarded above"),
            },
            QuoteStyle::Shell => match value {
                ConfigValue::String(s) => shell_quote(s, self.dialect.comment_char),
                ConfigValue::Bool(b) => b.to_string(),
                ConfigValue::Int(i) => i.to_string(),
                ConfigValue::Float(f) => format_float(*f),
                ConfigValue::Opaque => unreachable!("guarded above"),
            },
            QuoteStyle::JsLiteral => match value {
                ConfigValue::String(s) => js_quote(s),
                ConfigValue::Bool(b) => b.to_string(),
                ConfigValue::Int(i) => i.to_string(),
                ConfigValue::Float(f) => format_float(*f),
                ConfigValue::Opaque => unreachable!("guarded above"),
            },
        })
    }

    /// Re-emit the line vector to text, restoring the original trailing-newline
    /// state. `force_trailing` is reserved for callers that always want a final
    /// newline; the format handlers pass `false` so the document's own ending is
    /// reproduced.
    fn emit(&self, lines: &[Line], force_trailing: bool) -> String {
        let mut out = String::new();
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            match line {
                Line::Section { raw, .. } | Line::Other { raw } => out.push_str(raw),
                Line::KeyVal(kv) => out.push_str(&kv.raw),
            }
        }
        if (self.trailing_newline || force_trailing) && !lines.is_empty() {
            out.push('\n');
        }
        out
    }
}

/// Split text into lines WITHOUT the trailing newline characters, preserving a
/// trailing empty segment only when the text ends without a newline (so a file
/// not ending in `\n` round-trips). A leading/standalone `\r` is folded so a CRLF
/// document re-emits with LF (a deliberate, documented normalization for the
/// hand-rolled formats; the value-bearing content is untouched).
fn split_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let normalized_end = text.strip_suffix('\n').unwrap_or(text);
    normalized_end
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

/// Parse a `[section]` header line (already trimmed of leading whitespace),
/// returning the section name. A `[` with no closing `]` is not a header (it
/// falls through to [`Line::Other`]).
fn parse_section_header(trimmed: &str) -> Option<String> {
    let rest = trimmed.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(rest[..end].trim().to_string())
}

/// Parse a modelled key line under `dialect`, returning a [`KeyVal`] with the
/// value span located in `raw`. Returns `None` when the line is not a modelled
/// key (so the caller files it as [`Line::Other`]).
fn parse_key_line(raw: &str, dialect: &LineDialect, section: Option<&str>) -> Option<KeyVal> {
    match &dialect.statement {
        Some(stmt) => parse_statement_line(raw, stmt),
        None => parse_plain_key_line(raw, dialect, section),
    }
}

/// Parse a plain `key<sep>value` line (INI / `.env` / flat). The key is
/// everything before the first separator (trimmed); the value run is from the
/// first non-space after the separator to the end of the value (an inline `#`
/// comment, where the dialect supports it, ends the value).
fn parse_plain_key_line(raw: &str, dialect: &LineDialect, section: Option<&str>) -> Option<KeyVal> {
    // Find the earliest separator on the line.
    let sep_pos = dialect
        .separators
        .iter()
        .filter_map(|&c| raw.find(c))
        .min()?;
    let key_part = raw[..sep_pos].trim();
    // A `.env` line may use `export KEY=...`; strip the prefix for the key but
    // keep it in `raw` so the rewrite preserves it.
    let key = key_part.strip_prefix("export ").unwrap_or(key_part).trim();
    if key.is_empty() {
        return None;
    }

    // The value run starts after the separator, skipping leading spaces.
    let after_sep = sep_pos + raw[sep_pos..].chars().next().map(|c| c.len_utf8())?;
    let value_region = &raw[after_sep..];
    let leading_ws = value_region.len() - value_region.trim_start().len();
    let value_start = after_sep + leading_ws;

    // The value end: for shell-quoted (.env) and bare values an inline comment
    // (` #` after a space) ends the value. A quoted value's comment detection
    // respects the closing quote.
    let value_end = locate_value_end(raw, value_start, dialect);
    let raw_value = raw[value_start..value_end].trim_end();
    let value_end = value_start + raw_value.len();

    let value = parse_scalar(raw_value, dialect);

    let local_key = key.to_string();
    let full_key = match section {
        Some(s) => format!("{s}.{local_key}"),
        None => local_key,
    };

    Some(KeyVal {
        raw: raw.to_string(),
        key: full_key,
        value,
        value_span: value_start..value_end,
    })
}

/// Find the byte index where a value run ends, honouring an inline comment for
/// dialects that allow one. For a value that opens with a quote, the closing
/// quote bounds it and any `#` inside is literal.
fn locate_value_end(raw: &str, value_start: usize, dialect: &LineDialect) -> usize {
    let region = &raw[value_start..];
    let bytes = region.as_bytes();
    if bytes.is_empty() {
        return value_start;
    }

    // Quoted value: scan to the matching unescaped closing quote, then the line
    // ends at the quote (anything after is trailing decor we leave on the line).
    let first = region.chars().next().unwrap();
    if (dialect.quote == QuoteStyle::Shell || dialect.quote == QuoteStyle::Bare)
        && (first == '"' || first == '\'')
    {
        let mut escaped = false;
        let mut idx = first.len_utf8();
        while idx < region.len() {
            let c = region[idx..].chars().next().unwrap();
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == first {
                return value_start + idx + c.len_utf8();
            }
            idx += c.len_utf8();
        }
        // Unterminated quote: take the rest of the line.
        return raw.len();
    }

    // Unquoted: an inline ` <comment_char>` (comment char preceded by space)
    // ends the value for the comment-bearing dialects.
    let comment = dialect.comment_char;
    let mut prev_space = false;
    let mut idx = 0usize;
    for c in region.chars() {
        if c == comment && prev_space {
            return value_start + idx;
        }
        prev_space = c == ' ' || c == '\t';
        idx += c.len_utf8();
    }
    raw.len()
}

/// Parse a `user_pref("key", value);` statement line, returning a [`KeyVal`]
/// with the value span located in `raw`. The value run is the token between the
/// comma and the closing `)`. The Firefox key contains dots and is used verbatim
/// as the key-path (not section-scoped, not re-split).
fn parse_statement_line(raw: &str, stmt: &StatementShape) -> Option<KeyVal> {
    let trimmed = raw.trim_start();
    let lead_ws = raw.len() - trimmed.len();
    let after_fn = trimmed.strip_prefix(stmt.func)?;
    let after_paren = after_fn.trim_start().strip_prefix('(')?;
    // Offset of `after_paren` within `raw`.
    let paren_open = lead_ws + (trimmed.len() - after_paren.len());

    // The key: a double-quoted string immediately (after optional whitespace).
    let key_region = after_paren.trim_start();
    let key_quote = key_region.strip_prefix('"')?;
    let key_end_rel = find_unescaped_quote(key_quote)?;
    let key = unescape_js(&key_quote[..key_end_rel]);
    // Position just past the key's closing quote, in `raw` coordinates.
    let key_close_abs = paren_open
        + (after_paren.len() - key_region.len())
        + 1
        + key_end_rel
        + 1;

    // Skip whitespace + the comma after the key.
    let post_key = &raw[key_close_abs..];
    let comma_rel = post_key.find(',')?;
    let value_region_start = key_close_abs + comma_rel + 1;

    // The value: from the first non-space after the comma to the matching `)`.
    let value_region = &raw[value_region_start..];
    let leading_ws = value_region.len() - value_region.trim_start().len();
    let value_start = value_region_start + leading_ws;

    let close_rel = find_statement_value_end(&raw[value_start..])?;
    let value_end = value_start + close_rel;
    let raw_value = raw[value_start..value_end].trim_end();
    let value_end = value_start + raw_value.len();

    let value = parse_js_scalar(raw_value);

    Some(KeyVal {
        raw: raw.to_string(),
        key,
        value,
        value_span: value_start..value_end,
    })
}

/// Find the byte index, within a value region that begins at the value token,
/// of the `)` that closes the statement, honouring a quoted string so a `)`
/// inside a string literal does not end the value early.
fn find_statement_value_end(region: &str) -> Option<usize> {
    let mut idx = 0usize;
    let bytes_len = region.len();
    let first = region.chars().next()?;
    if first == '"' || first == '\'' {
        // Quoted value: end at the char after the closing quote.
        let inner = &region[first.len_utf8()..];
        let close = find_unescaped_quote_of(inner, first)?;
        return Some(first.len_utf8() + close + first.len_utf8());
    }
    // Bare literal (bool/number): end at the first `)` or `;` or `,`.
    while idx < bytes_len {
        let c = region[idx..].chars().next().unwrap();
        if c == ')' || c == ';' {
            return Some(idx);
        }
        idx += c.len_utf8();
    }
    None
}

/// Find the byte index of the first unescaped `"` in a string-literal body
/// (the body after an opening double quote).
fn find_unescaped_quote(s: &str) -> Option<usize> {
    find_unescaped_quote_of(s, '"')
}

/// Find the byte index of the first unescaped `quote` char in `s`.
fn find_unescaped_quote_of(s: &str, quote: char) -> Option<usize> {
    let mut escaped = false;
    let mut idx = 0usize;
    for c in s.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            return Some(idx);
        }
        idx += c.len_utf8();
    }
    None
}

/// Parse a scalar value token from an INI/.env/flat value run.
fn parse_scalar(raw_value: &str, dialect: &LineDialect) -> ConfigValue {
    // Strip surrounding quotes for shell/bare-quoted values.
    let unquoted = strip_surrounding_quotes(raw_value);
    // Only treat as bool/int/float when the value was NOT quoted (a quoted
    // "true" is a string by intent).
    let was_quoted = unquoted.len() != raw_value.len();
    if !was_quoted {
        if let Some(v) = parse_bool(unquoted) {
            return ConfigValue::Bool(v);
        }
        if let Ok(i) = unquoted.parse::<i64>() {
            return ConfigValue::Int(i);
        }
        if let Some(f) = parse_float(unquoted) {
            return ConfigValue::Float(f);
        }
    }
    let _ = dialect;
    ConfigValue::String(unescape_shell(unquoted))
}

/// Parse a JS literal value token from a `user_pref` statement.
fn parse_js_scalar(raw_value: &str) -> ConfigValue {
    if let Some(stripped) = strip_js_string(raw_value) {
        return ConfigValue::String(unescape_js(stripped));
    }
    if let Some(b) = parse_bool(raw_value) {
        return ConfigValue::Bool(b);
    }
    if let Ok(i) = raw_value.parse::<i64>() {
        return ConfigValue::Int(i);
    }
    if let Some(f) = parse_float(raw_value) {
        return ConfigValue::Float(f);
    }
    ConfigValue::String(raw_value.to_string())
}

/// Parse the booleans `true`/`false`, case-sensitive (config bools are
/// lowercase across all six formats).
fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Parse a finite float, rejecting the non-finite and the integer-shaped (an
/// integer is classified as [`ConfigValue::Int`] first by the caller). Rejects
/// inf/NaN so a hostile literal cannot smuggle a non-finite value into the model.
fn parse_float(s: &str) -> Option<f64> {
    // Must contain a `.`, `e`/`E` to be a float and not an int that overflowed.
    if !s.contains(['.', 'e', 'E']) {
        return None;
    }
    match s.parse::<f64>() {
        Ok(f) if f.is_finite() => Some(f),
        _ => None,
    }
}

/// Strip a single pair of matching surrounding `"` or `'` quotes, if present.
fn strip_surrounding_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Strip a single pair of surrounding double quotes (a JS string), returning the
/// inner body, or `None` if the token is not a double-quoted string.
fn strip_js_string(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 && bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"' {
        Some(&s[1..s.len() - 1])
    } else {
        None
    }
}

/// Quote a string for `.env` only when it needs it (whitespace, a quote, the
/// comment char, or an `=`). A value with none of those is written bare so the
/// common case stays clean.
fn shell_quote(s: &str, comment_char: char) -> String {
    let needs = s.is_empty()
        || s.chars()
            .any(|c| c.is_whitespace() || c == '"' || c == '\'' || c == comment_char || c == '=');
    if !needs {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Quote a string as a JS double-quoted literal, escaping `"`, `\`, and the
/// control chars that would break a single-line `user_pref` statement.
fn js_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Reverse [`shell_quote`]'s escaping for a value parsed out of a quoted run.
fn unescape_shell(s: &str) -> String {
    unescape_with(s, |c| match c {
        '"' => Some('"'),
        '\\' => Some('\\'),
        _ => None,
    })
}

/// Reverse [`js_quote`]'s escaping for a value parsed out of a JS string.
fn unescape_js(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('u') => {
                // \uXXXX: take four hex digits; pass through literally on a
                // malformed escape (the value is data, never re-executed).
                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                match u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    Some(ch) => out.push(ch),
                    None => {
                        out.push('\\');
                        out.push('u');
                        out.push_str(&hex);
                    }
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Generic backslash-unescaper for the simple two-char escapes.
fn unescape_with(s: &str, map: impl Fn(char) -> Option<char>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                if let Some(mapped) = map(next) {
                    out.push(mapped);
                    chars.next();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// Format a finite float compactly: prefer the shortest round-tripping form, and
/// always keep a `.0` so an integer-valued float is not re-read as an int.
fn format_float(f: f64) -> String {
    let s = format!("{f}");
    if s.contains(['.', 'e', 'E']) {
        s
    } else {
        format!("{s}.0")
    }
}

/// Split a dotted key-path into `(section, local_key)` on the first `.`. INI
/// key-paths are exactly two segments (`section.key`); a deeper path keeps the
/// remainder as the local key verbatim (the engine does not nest beyond one
/// section level).
fn split_section_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ini_dialect() -> LineDialect {
        LineDialect {
            sections: true,
            separators: &['=', ':'],
            insert_separator: " = ",
            comment_char: '#',
            quote: QuoteStyle::Bare,
            statement: None,
        }
    }

    #[test]
    fn parses_and_serializes_a_float_with_dot() {
        assert_eq!(format_float(1.5), "1.5");
        assert_eq!(format_float(2.0), "2.0");
        assert_eq!(parse_float("1.5"), Some(1.5));
        assert_eq!(parse_float("42"), None, "an int is not a float");
        assert_eq!(parse_float("inf"), None, "non-finite rejected");
        assert_eq!(parse_float("NaN"), None, "non-finite rejected");
    }

    #[test]
    fn split_lines_round_trips_trailing_newline_state() {
        assert_eq!(split_lines("a\nb\n"), vec!["a", "b"]);
        assert_eq!(split_lines("a\nb"), vec!["a", "b"]);
        assert!(split_lines("").is_empty());
    }

    #[test]
    fn ini_read_collects_section_scoped_keys() {
        let model = LineModel::parse("[a]\nx = 1\ny = hi\n", ini_dialect()).unwrap();
        let m = model.to_model();
        assert_eq!(m.get("a.x"), Some(&ConfigValue::Int(1)));
        assert_eq!(m.get("a.y"), Some(&ConfigValue::String("hi".to_string())));
    }
}
