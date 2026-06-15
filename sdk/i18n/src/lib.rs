//! Arlen internationalization (i18n-plan.md I18N-R1).
//!
//! MessageFormat 2.0 (Unicode CLDR 47, the frozen spec) is Arlen's message
//! format, one catalog shared by the Rust daemons and the Svelte frontend. This
//! crate is the Rust side: the [`model`] (the spec-shaped MF2 data model), the
//! [`parse`]r, and (building on them) the formatter + catalog loading. The
//! formatter is in-house over ICU4X's plural/number/locale primitives until
//! ICU4X ships its own MF2 formatter; the catalogs are stable through that swap,
//! so the swap is an implementation change, not a content rewrite.

pub mod format;
pub mod model;
pub mod parse;

pub use format::{format, ArgValue, Args};
pub use model::Message;
pub use parse::{parse_message, ParseError};
