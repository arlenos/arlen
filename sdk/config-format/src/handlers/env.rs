//! `.env` handler: line-oriented `KEY=value`, preserving `export `, surrounding
//! quotes, inline `#` comments, blank lines and ordering.
//!
//! Backed by the shared [`crate::line_model`] engine with shell-style quoting (a
//! value is quoted on rewrite only when it needs it). `dotenvy` is a loader, not
//! a format-preserving editor, so it is not used.

use crate::error::{EditError, ParseError};
use crate::line_model::{LineDialect, LineModel, QuoteStyle};
use crate::model::{ConfigModel, ConfigValue};
use crate::{Format, FormatHandler};

/// The `.env` format handler.
#[derive(Debug, Default, Clone, Copy)]
pub struct EnvHandler;

/// The `.env` dialect: no sections, `=` separator, `#` comments, shell quoting.
fn dialect() -> LineDialect {
    LineDialect {
        sections: false,
        separators: &['='],
        insert_separator: "=",
        comment_char: '#',
        quote: QuoteStyle::Shell,
        statement: None,
    }
}

impl FormatHandler for EnvHandler {
    fn format(&self) -> Format {
        Format::Env
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
    fn read_keeps_export_prefix_keys() {
        let h = EnvHandler;
        let m = h.read("export PATH=/usr/bin\nLANG=en_US\n").unwrap();
        assert_eq!(
            m.get("PATH"),
            Some(&ConfigValue::String("/usr/bin".to_string()))
        );
        assert_eq!(
            m.get("LANG"),
            Some(&ConfigValue::String("en_US".to_string()))
        );
    }

    #[test]
    fn set_preserves_comments_and_export() {
        let h = EnvHandler;
        let src = "\
# database
DB_HOST=localhost
export DB_PORT=5432
";
        let out = h.set(src, "DB_PORT", &ConfigValue::Int(6543)).unwrap();
        assert!(out.contains("# database"));
        assert!(out.contains("DB_HOST=localhost"));
        assert!(
            out.contains("export DB_PORT=6543"),
            "export prefix kept; got:\n{out}"
        );
    }
}
