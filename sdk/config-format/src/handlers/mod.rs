//! The six built-in format handlers.
//!
//! Each module implements [`FormatHandler`](crate::FormatHandler) for one
//! config format. The four line-oriented formats (INI, `.env`, flat
//! `key=value`, Firefox `prefs.js`) share the [`crate::line_model`] engine; TOML
//! and JSON keep their CST dependencies (`toml_edit`, `jsonc-parser`).

pub mod env;
pub mod firefox;
pub mod flat;
pub mod ini;
pub mod json;
pub mod toml;
