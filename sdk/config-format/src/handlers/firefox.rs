//! Firefox `prefs.js` handler: the `user_pref("key", value);` line model.
//!
//! Each modelled line matches `user_pref(\s*"<key>"\s*,\s*<value>\s*);`. The key
//! already contains dots (`browser.startup.homepage`) and is the key-path
//! verbatim, NOT re-split on `.`. A `set` rewrites the value token inside the
//! statement; an insert appends a new `user_pref("k", v);` line. Non-`user_pref`
//! lines (comments, `pref(...)`, blank) are unmodelled and preserved.
//!
//! Backed by the shared [`crate::line_model`] engine with the `user_pref`
//! statement shape and JS-literal value quoting.

use crate::error::{EditError, ParseError};
use crate::line_model::{LineDialect, LineModel, QuoteStyle, StatementShape};
use crate::model::{ConfigModel, ConfigValue};
use crate::{Format, FormatHandler};

/// The Firefox `prefs.js` format handler.
#[derive(Debug, Default, Clone, Copy)]
pub struct FirefoxPrefsHandler;

/// The `prefs.js` dialect: no INI sections, the `user_pref` statement shape,
/// `#` is not a comment char here (`//` line comments are handled in the engine),
/// JS-literal value quoting.
fn dialect() -> LineDialect {
    LineDialect {
        sections: false,
        // The separator/insert are unused for a statement dialect (the engine
        // routes through the statement parser and `format_key_line`'s statement
        // branch), but the struct requires values.
        separators: &[','],
        insert_separator: ", ",
        comment_char: '#',
        quote: QuoteStyle::JsLiteral,
        statement: Some(StatementShape { func: "user_pref" }),
    }
}

impl FormatHandler for FirefoxPrefsHandler {
    fn format(&self) -> Format {
        Format::FirefoxPrefs
    }

    fn read(&self, text: &str) -> Result<ConfigModel, ParseError> {
        Ok(LineModel::parse(text, dialect())?.to_model())
    }

    fn set(&self, text: &str, key: &str, value: &ConfigValue) -> Result<String, EditError> {
        let model = LineModel::parse(text, dialect()).map_err(parse_to_edit)?;
        model.set(key, value).map_err(EditError::Failed)
    }

    fn remove(&self, text: &str, key: &str) -> Result<String, EditError> {
        let model = LineModel::parse(text, dialect()).map_err(parse_to_edit)?;
        model.remove(key).map_err(EditError::Failed)
    }
}

fn parse_to_edit(e: ParseError) -> EditError {
    EditError::Failed(format!("parse: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_collects_user_pref_keys_verbatim() {
        let h = FirefoxPrefsHandler;
        let src = "\
// Mozilla User Preferences
user_pref(\"browser.startup.homepage\", \"https://arlen.os\");
user_pref(\"browser.startup.page\", 1);
user_pref(\"privacy.donottrackheader.enabled\", true);
";
        let m = h.read(src).unwrap();
        assert_eq!(
            m.get("browser.startup.homepage"),
            Some(&ConfigValue::String("https://arlen.os".to_string()))
        );
        assert_eq!(m.get("browser.startup.page"), Some(&ConfigValue::Int(1)));
        assert_eq!(
            m.get("privacy.donottrackheader.enabled"),
            Some(&ConfigValue::Bool(true))
        );
    }

    #[test]
    fn set_rewrites_only_the_value_token() {
        let h = FirefoxPrefsHandler;
        let src = "\
// header
user_pref(\"browser.startup.page\", 1);
user_pref(\"browser.startup.homepage\", \"https://old\");
";
        let out = h
            .set(
                src,
                "browser.startup.homepage",
                &ConfigValue::String("https://new".to_string()),
            )
            .unwrap();
        assert!(out.contains("// header"));
        assert!(out.contains("user_pref(\"browser.startup.page\", 1);"));
        assert!(
            out.contains("user_pref(\"browser.startup.homepage\", \"https://new\");"),
            "value rewritten; got:\n{out}"
        );
        assert!(!out.contains("https://old"));
    }
}
