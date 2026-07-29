//! PAS-6 tier 2: validating what the user typed into the raw TOML editor.
//!
//! A schema marks one key `raw` when its value is something the type vocabulary
//! cannot express. Settings then shows that key's value as text and lets the user
//! edit it directly, which is the boring, universal answer VS Code and Chrome
//! policy both arrived at.
//!
//! **Validated means two things, and the second is the one that matters.** The
//! text has to parse as TOML, and whatever it parses to becomes the value of that
//! ONE declared key. It is not a patch applied to the config file. A user who
//! types `[network]` into the raw box for `advanced.tuning` gets a table nested
//! under `advanced.tuning`, not a new top-level `network` section - so a raw item
//! cannot be talked into writing a key the schema never declared, and every scope
//! rule on the declared key still applies.

/// The most text the raw editor will accept for one value.
///
/// A config file is hand-sized by definition. The cap is here because this is
/// paste-shaped input landing in a file the app parses at every start, and
/// nothing legitimate about one setting's value runs to tens of kilobytes.
pub const MAX_RAW_BYTES: usize = 64 * 1024;

/// Why a raw edit was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawEditError {
    /// Longer than [`MAX_RAW_BYTES`].
    TooLarge {
        /// What was submitted.
        bytes: usize,
    },
    /// It does not parse as TOML. Carries the parser's own message, which names
    /// the line and is far more use than "invalid".
    NotToml(String),
    /// Nothing was typed. Clearing a value is a removal, not a write of empty.
    Empty,
}

impl std::fmt::Display for RawEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RawEditError::TooLarge { bytes } => write!(
                f,
                "that is {bytes} bytes; one setting's value may be at most {MAX_RAW_BYTES}"
            ),
            RawEditError::NotToml(msg) => write!(f, "that is not valid TOML: {msg}"),
            RawEditError::Empty => write!(f, "nothing to save"),
        }
    }
}

impl std::error::Error for RawEditError {}

/// Turn the text in the raw editor into the value for one key.
///
/// Two shapes are accepted because a raw value is not always a table. Text that
/// parses as a TOML document becomes that table; text that does not is tried
/// once more as a bare value, which is what `[1, 2, 3]` or `"a string"` needs.
/// Trying the document form first matters: `[1, 2, 3]` is also a (nonsense)
/// document header, and the value reading is the one the user meant.
pub fn parse_raw_edit(text: &str) -> Result<toml::Value, RawEditError> {
    if text.len() > MAX_RAW_BYTES {
        return Err(RawEditError::TooLarge { bytes: text.len() });
    }
    if text.trim().is_empty() {
        return Err(RawEditError::Empty);
    }

    // A bare value is the narrower reading, so try it first and fall back to the
    // document form. `[1, 2, 3]` parses as a value (an array) and as a document
    // header; the array is what someone typing it into a value box meant.
    if let Ok(value) = parse_as_value(text) {
        return Ok(value);
    }

    match text.parse::<toml::Table>() {
        Ok(table) => Ok(toml::Value::Table(table)),
        // Report the document error, not the value one: a multi-line edit is the
        // common case, and its message names the offending line.
        Err(e) => Err(RawEditError::NotToml(first_line(&e.to_string()))),
    }
}

/// Parse text as a single TOML value by giving it a key to belong to.
fn parse_as_value(text: &str) -> Result<toml::Value, ()> {
    // A newline in the wrapper would end the key-value pair, so a multi-line
    // edit can never be read as a bare value - which is correct, it is a
    // document.
    if text.contains('\n') {
        return Err(());
    }
    let wrapped = format!("v = {}", text.trim());
    match wrapped.parse::<toml::Table>() {
        Ok(t) => t.get("v").cloned().ok_or(()),
        Err(_) => Err(()),
    }
}

/// Render a value back into the text the editor shows.
///
/// A table is shown as a document (`a = 1` per line) rather than an inline
/// table, because that is the shape the user will type back and the shape the
/// rest of their config file is written in.
pub fn render_raw_value(value: &toml::Value) -> String {
    match value {
        toml::Value::Table(t) => toml::to_string_pretty(t).unwrap_or_default(),
        other => other.to_string(),
    }
}

/// The first line of a parser message: the rest is a source excerpt that means
/// nothing without the surrounding file.
fn first_line(message: &str) -> String {
    message.lines().next().unwrap_or(message).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property the whole tier rests on: a raw edit becomes the value of its
    /// own key. Typing a section header cannot create a top-level key the schema
    /// never declared.
    #[test]
    fn a_section_header_lands_inside_the_edited_key() {
        let value = parse_raw_edit("[network]\nproxy = \"http://x\"\n").unwrap();
        let table = value.as_table().expect("a document is a table");
        assert!(table.contains_key("network"));
        // What matters is that this is a value to be stored AT the declared key.
        // Nothing here is a top-level write; the broker puts it where the schema
        // says, and the schema said one key.
        assert_eq!(
            table["network"]["proxy"].as_str(),
            Some("http://x"),
            "the typed section is nested in the value"
        );
    }

    #[test]
    fn a_multi_line_document_becomes_a_table() {
        let value = parse_raw_edit("retries = 3\ntimeout = \"30s\"\n").unwrap();
        assert_eq!(value["retries"].as_integer(), Some(3));
        assert_eq!(value["timeout"].as_str(), Some("30s"));
    }

    /// Not every raw value is a table: an array or a scalar has to survive too.
    #[test]
    fn a_bare_array_stays_an_array() {
        let value = parse_raw_edit("[1, 2, 3]").unwrap();
        let array = value.as_array().expect("should be an array, not a header");
        assert_eq!(array.len(), 3);
    }

    #[test]
    fn a_bare_scalar_survives() {
        assert_eq!(parse_raw_edit("\"hello\"").unwrap().as_str(), Some("hello"));
        assert_eq!(parse_raw_edit("42").unwrap().as_integer(), Some(42));
        assert_eq!(parse_raw_edit("true").unwrap().as_bool(), Some(true));
    }

    /// The parser message names the line, which is the whole reason to pass it
    /// through instead of saying "invalid".
    #[test]
    fn a_syntax_error_reports_what_the_parser_said() {
        let err = parse_raw_edit("a = = 1").unwrap_err();
        match err {
            RawEditError::NotToml(msg) => assert!(!msg.is_empty(), "should carry a message"),
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    /// Clearing the box is a removal, not a write of nothing.
    #[test]
    fn an_empty_edit_is_refused() {
        assert_eq!(parse_raw_edit("   \n  ").unwrap_err(), RawEditError::Empty);
        assert_eq!(parse_raw_edit("").unwrap_err(), RawEditError::Empty);
    }

    #[test]
    fn an_oversized_edit_is_refused_before_parsing() {
        let huge = "x = 1\n".repeat(MAX_RAW_BYTES);
        match parse_raw_edit(&huge).unwrap_err() {
            RawEditError::TooLarge { bytes } => assert_eq!(bytes, huge.len()),
            other => panic!("expected a size refusal, got {other:?}"),
        }
    }

    /// What the editor shows must be what it will accept back, or the first save
    /// of an untouched value fails.
    #[test]
    fn what_is_rendered_parses_back_to_the_same_value() {
        for text in [
            "retries = 3\ntimeout = \"30s\"\n",
            "[1, 2, 3]",
            "\"hello\"",
            "[nested]\nkey = true\n",
        ] {
            let value = parse_raw_edit(text).unwrap();
            let rendered = render_raw_value(&value);
            let round_tripped = parse_raw_edit(&rendered)
                .unwrap_or_else(|e| panic!("{text:?} rendered to {rendered:?} which fails: {e}"));
            assert_eq!(round_tripped, value, "round trip changed {text:?}");
        }
    }
}
