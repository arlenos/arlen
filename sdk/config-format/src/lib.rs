//! Format-preserving config handlers for the Arlen integration packages.
//!
//! A single [`FormatHandler`] trait reads a config document into a
//! format-agnostic [`ConfigModel`] (every setting addressed by one dotted
//! [`KeyPath`]) and edits the document AS TEXT, so comments, blank lines,
//! ordering and the exact whitespace of untouched lines survive a write. Six
//! handlers implement it: TOML, JSON/JSONC, INI/`.conf`, Firefox `prefs.js`,
//! `.env`, and flat `key=value` (see [`handlers`]).
//!
//! Every edit goes through the mandatory read-after-write self-check
//! ([`checked_set`], [`checked_remove`]): the candidate text is re-parsed and
//! three assertions are run before a byte is returned (the target key took, no
//! sibling changed, no comment or formatting outside the target was rewritten).
//! On any failure the original text is returned unchanged, so a checked edit
//! never hands back lossy text.
//!
//! Untrusted third-party config is read through the [`confined`] boundary:
//! cap-std confines the read to a capability root (no `..` or absolute escape)
//! and the bytes are stripped to inert text in the S18-B parse sandbox before a
//! handler ever parses them.

#![warn(missing_docs)]

pub mod confined;
pub mod error;
pub mod handlers;
pub mod line_model;
pub mod model;
pub mod selfcheck;

pub use error::{EditError, ParseError, SelfCheckError};
pub use model::{ConfigModel, ConfigValue, KeyPath};
pub use selfcheck::{checked_remove, checked_set};

/// The largest config document accepted by any handler. Oversize input is
/// refused outright (not truncated and parsed), since a truncated config is a
/// wrong model. Shared so the line-model engine and the CST parsers agree on one
/// bound.
pub const MAX_CONFIG_BYTES: usize = 4 * 1024 * 1024;

/// The deepest nesting a structured parser (TOML/JSON) will walk before
/// refusing the document. Guards the CST-based readers against a
/// stack-exhausting input.
pub const MAX_DEPTH: usize = 64;

/// The config formats this library handles. A handler reports its own format
/// through [`FormatHandler::format`]; the variant also drives format selection
/// at the [`confined`] boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// TOML, via `toml_edit`.
    Toml,
    /// JSON / JSONC, via `jsonc-parser`'s comment-preserving CST.
    Json,
    /// INI / `.conf` (sectioned `key = value`).
    Ini,
    /// Firefox `prefs.js` (`user_pref("key", value);`).
    FirefoxPrefs,
    /// `.env` (`KEY=value`, shell quoting, optional `export`).
    Env,
    /// Flat `key=value` / `key: value` (single level, no sections).
    Flat,
}

/// A format handler: reads a config document into a [`ConfigModel`] and edits it
/// as text without losing comments or formatting.
///
/// The trait is object-safe so the self-check ([`checked_set`] /
/// [`checked_remove`]) can drive any handler through `&dyn FormatHandler`. The
/// three operations are total and panic-free on adversarial input: a malformed
/// document yields a [`ParseError`] (read) or an [`EditError`] (set/remove),
/// never a panic.
///
/// Editing operates on TEXT, not on the [`ConfigModel`], because
/// format-preservation needs the original bytes: serializing a model back to
/// text would drop comments and whitespace. `read` is only ever used to inspect
/// values and to drive the self-check.
pub trait FormatHandler {
    /// The format this handler speaks.
    fn format(&self) -> Format;

    /// Read `text` into a [`ConfigModel`]: every modelled scalar key-path paired
    /// with its value, in document order. Returns a [`ParseError`] for oversize,
    /// malformed, over-deep or non-UTF-8 input.
    fn read(&self, text: &str) -> Result<ConfigModel, ParseError>;

    /// Produce a candidate document with `key` set to `value`, preserving every
    /// comment and the surrounding formatting. A new key is inserted in the
    /// natural place for the format. Returns an [`EditError`] if the document
    /// does not parse, the path cannot be addressed, or the existing value at
    /// the path is non-scalar ([`ConfigValue::Opaque`]).
    ///
    /// This is the raw edit primitive: callers should prefer [`checked_set`],
    /// which runs the read-after-write self-check over the candidate this
    /// returns.
    fn set(&self, text: &str, key: &str, value: &ConfigValue) -> Result<String, EditError>;

    /// Produce a candidate document with `key` removed, preserving every other
    /// line. An absent key is a no-op that returns the text unchanged. Returns an
    /// [`EditError`] only if the document does not parse.
    ///
    /// This is the raw edit primitive: callers should prefer [`checked_remove`],
    /// which runs the read-after-write self-check over the candidate this
    /// returns.
    fn remove(&self, text: &str, key: &str) -> Result<String, EditError>;
}

/// The handler for `format`, as a boxed trait object. The single place that maps
/// a [`Format`] to its concrete handler, so a caller (and the [`confined`]
/// boundary) selects one by enum rather than naming the concrete type.
pub fn handler_for(format: Format) -> Box<dyn FormatHandler> {
    match format {
        Format::Toml => Box::new(handlers::toml::TomlHandler),
        Format::Json => Box::new(handlers::json::JsonHandler),
        Format::Ini => Box::new(handlers::ini::IniHandler),
        Format::FirefoxPrefs => Box::new(handlers::firefox::FirefoxPrefsHandler),
        Format::Env => Box::new(handlers::env::EnvHandler),
        Format::Flat => Box::new(handlers::flat::FlatHandler),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_for_reports_its_own_format() {
        for format in [
            Format::Toml,
            Format::Json,
            Format::Ini,
            Format::FirefoxPrefs,
            Format::Env,
            Format::Flat,
        ] {
            assert_eq!(handler_for(format).format(), format);
        }
    }

    #[test]
    fn checked_set_drives_a_handler_through_the_trait_object() {
        let handler = handler_for(Format::Toml);
        let out = checked_set(
            handler.as_ref(),
            "port = 8080\n",
            "port",
            &ConfigValue::Int(9090),
        )
        .unwrap();
        assert!(out.contains("port = 9090"));
    }
}
