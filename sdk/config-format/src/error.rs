//! The error types for the format-handler library: parse failures, edit
//! failures, and the read-after-write self-check failures that gate every edit.

use thiserror::Error;

/// A parse failure. Returned by [`FormatHandler::read`](crate::FormatHandler::read)
/// for adversarial or malformed input; never a panic. Parsing is total: every
/// input either yields a [`ConfigModel`](crate::ConfigModel) or one of these.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The input exceeded the byte cap ([`crate::MAX_CONFIG_BYTES`]). Oversize
    /// input is refused (not truncated and parsed), since a truncated config is
    /// a wrong model.
    #[error("config too large")]
    TooLarge,

    /// The bytes were not valid for the format (a structural error: an
    /// unterminated string, a `[` with no `]`, a JSON syntax error the parser
    /// rejected outright, etc.).
    #[error("malformed {format}: {detail}")]
    Malformed {
        /// The format that failed to parse.
        format: &'static str,
        /// A short human description of the structural problem.
        detail: String,
    },

    /// The document nested past the depth cap ([`crate::MAX_DEPTH`]). Guards the
    /// CST-based parsers (JSON) against a stack-exhausting input.
    #[error("config nested too deeply (> {0})")]
    TooDeep(usize),

    /// The input bytes were not valid UTF-8 on a path that requires it (the
    /// in-process trusted read; the sandboxed path bounds this earlier).
    #[error("config is not valid UTF-8")]
    NotUtf8,
}

/// A failed self-check. Every variant means the candidate edit was REJECTED and
/// the original text is returned unchanged: a checked edit never returns lossy
/// text. Surfaced through [`EditError::SelfCheck`].
#[derive(Debug, Error, PartialEq)]
pub enum SelfCheckError {
    /// The candidate text no longer parses. A buggy edit that corrupts the
    /// document fails closed here rather than being written.
    #[error("the edited document no longer parses: {0}")]
    CandidateUnparsable(String),

    /// The target key does not hold the intended value after the edit (for a
    /// `set`), or is still present (for a `remove`). The edit did not take.
    #[error("the edit did not apply to {key}")]
    EditDidNotApply {
        /// The key the edit targeted.
        key: String,
    },

    /// Some OTHER modelled key was added, removed, or changed by the edit. The
    /// edit had collateral effect on a sibling setting.
    #[error("the edit changed an unrelated key: {key}")]
    CollateralChange {
        /// The sibling key that changed.
        key: String,
    },

    /// The unmodelled bytes (comments, blank lines, unknown content) of the
    /// document changed beyond the single target key's own region. A comment or
    /// formatting was dropped or rewritten.
    #[error("the edit changed comments or formatting outside the target key")]
    UnmodelledContentChanged,
}

/// An edit failure. Returned by the checked edit entry points
/// ([`crate::checked_set`] / [`crate::checked_remove`]). Every variant means the
/// edit was refused; the caller keeps the original text.
#[derive(Debug, Error)]
pub enum EditError {
    /// The candidate text could not be produced because the document did not
    /// parse to begin with, or the target path could not be addressed in the
    /// format (a scalar key-path that runs through a non-table parent, an
    /// existing non-scalar at the path, etc.).
    #[error("edit failed: {0}")]
    Failed(String),

    /// The existing value at the target path is non-scalar
    /// ([`crate::ConfigValue::Opaque`]); refused rather than flattened.
    #[error("refusing to overwrite a non-scalar value at {key}")]
    OpaqueTarget {
        /// The key whose existing value is non-scalar.
        key: String,
    },

    /// The candidate produced by the inner edit failed the mandatory
    /// read-after-write self-check. The original text is returned unchanged.
    #[error(transparent)]
    SelfCheck(#[from] SelfCheckError),
}
