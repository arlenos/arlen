//! Flat `key=value` / `key: value` handler: the `.env` model minus shell quoting
//! and `export`. The generic single-level key/value config (a `.properties`-ish
//! or `.conf`-without-sections file).
//!
//! Backed by the shared [`crate::line_model`] engine with bare values and both
//! `=` and `:` accepted as separators.

use crate::error::{EditError, ParseError};
use crate::line_model::{LineDialect, LineModel, QuoteStyle};
use crate::model::{ConfigModel, ConfigValue};
use crate::{Format, FormatHandler};

/// The flat `key=value` format handler.
#[derive(Debug, Default, Clone, Copy)]
pub struct FlatHandler;

/// The flat dialect: no sections, `=`/`:` separators, `#` comments, bare values.
fn dialect() -> LineDialect {
    LineDialect {
        sections: false,
        separators: &['=', ':'],
        insert_separator: "=",
        comment_char: '#',
        quote: QuoteStyle::Bare,
        statement: None,
    }
}

impl FormatHandler for FlatHandler {
    fn format(&self) -> Format {
        Format::Flat
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
    fn read_collects_flat_keys() {
        let h = FlatHandler;
        let m = h.read("name = arlen\nretries = 3\n").unwrap();
        assert_eq!(
            m.get("name"),
            Some(&ConfigValue::String("arlen".to_string()))
        );
        assert_eq!(m.get("retries"), Some(&ConfigValue::Int(3)));
    }

    #[test]
    fn inserting_the_first_key_into_an_empty_config_succeeds() {
        // A fresh / empty config must accept its first key through the checked
        // path: the self-check treats empty text as zero lines, so the insert is
        // not wrongly rejected as a lost-content edit (the empty-file regression).
        let h = FlatHandler;
        let out =
            crate::selfcheck::checked_set(&h, "", "greeting", &ConfigValue::String("hi".into()))
                .expect("first insert into an empty config should pass the self-check");
        assert_eq!(
            h.read(&out).unwrap().get("greeting"),
            Some(&ConfigValue::String("hi".to_string()))
        );
    }

    #[test]
    fn set_preserves_comments() {
        let h = FlatHandler;
        let src = "# a flat config\nname = arlen\nretries = 3\n";
        let out = h.set(src, "retries", &ConfigValue::Int(5)).unwrap();
        assert!(out.contains("# a flat config"));
        assert!(out.contains("name = arlen"));
        assert!(out.contains("retries = 5"));
    }
}
