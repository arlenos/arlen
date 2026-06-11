//! INI / `.conf` handler: comment-, order- and duplicate-section-preserving.
//!
//! No Rust INI crate preserves comments + ordering + the write-back nuance
//! reliably, so this is the hand-rolled [`crate::line_model`] engine configured
//! with sections, `=`/`:` separators and `#` comments. A key-path is
//! `section.key`; a `set` rewrites only the matched key's value run, an insert
//! appends to the end of the matching section (creating it at document end if
//! absent).

use crate::error::{EditError, ParseError};
use crate::line_model::{LineDialect, LineModel, QuoteStyle};
use crate::model::{ConfigModel, ConfigValue};
use crate::{Format, FormatHandler};

/// The INI / `.conf` format handler.
#[derive(Debug, Default, Clone, Copy)]
pub struct IniHandler;

/// The INI dialect: sectioned, `=`/`:` separators, `#` comments, bare values.
fn dialect() -> LineDialect {
    LineDialect {
        sections: true,
        separators: &['=', ':'],
        insert_separator: " = ",
        comment_char: '#',
        quote: QuoteStyle::Bare,
        statement: None,
    }
}

impl FormatHandler for IniHandler {
    fn format(&self) -> Format {
        Format::Ini
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

/// Surface a parse failure encountered while preparing an edit as an
/// [`EditError::Failed`].
fn parse_to_edit(e: ParseError) -> EditError {
    EditError::Failed(format!("parse: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_collects_section_scoped_keys() {
        let h = IniHandler;
        let m = h
            .read("[server]\nhost = localhost\nport = 8080\n")
            .unwrap();
        assert_eq!(
            m.get("server.host"),
            Some(&ConfigValue::String("localhost".to_string()))
        );
        assert_eq!(m.get("server.port"), Some(&ConfigValue::Int(8080)));
    }

    #[test]
    fn set_preserves_comments_and_order() {
        let h = IniHandler;
        let src = "\
# top comment
[server]
# host comment
host = localhost
port = 8080
";
        let out = h
            .set(src, "server.port", &ConfigValue::Int(9090))
            .unwrap();
        assert!(out.contains("# top comment"));
        assert!(out.contains("# host comment"));
        assert!(out.contains("host = localhost"));
        assert!(out.contains("port = 9090"));
        // Order preserved: host before port.
        let host_at = out.find("host = localhost").unwrap();
        let port_at = out.find("port = 9090").unwrap();
        assert!(host_at < port_at);
    }
}
