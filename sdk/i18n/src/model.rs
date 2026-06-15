//! The MessageFormat 2.0 data model (i18n-plan.md I18N-R1).
//!
//! A parsed MF2 message is either a [`Message::Pattern`] (a sequence of text and
//! placeholders, optionally with declarations) or a [`Message::Select`] (a
//! matcher: declarations, one or more selectors, and the keyed variants). This
//! mirrors the MF2 spec's data model (the `PatternMessage` / `SelectMessage`
//! split); the [`crate::parse`] module builds it and [`crate::format`] renders
//! it. The model is deliberately spec-shaped so it survives the eventual swap to
//! ICU4X's own MF2 formatter (the catalogs and this model stay; only the
//! rendering backend changes).

/// A parsed MF2 message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// A simple/pattern message: declarations (often none) + one pattern.
    Pattern {
        /// `.input` / `.local` declarations evaluated before the pattern.
        declarations: Vec<Declaration>,
        /// The pattern to render.
        pattern: Pattern,
    },
    /// A select message: declarations, the `.match` selectors, and the variants.
    Select {
        /// `.input` / `.local` declarations.
        declarations: Vec<Declaration>,
        /// The selector expressions (`.match $a $b`).
        selectors: Vec<Expression>,
        /// The keyed variants; one is chosen per selection.
        variants: Vec<Variant>,
    },
}

/// An ordered sequence of literal text and placeholder expressions.
pub type Pattern = Vec<Part>;

/// One piece of a [`Pattern`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    /// Literal text (escapes already resolved).
    Text(String),
    /// A `{...}` placeholder expression.
    Expression(Expression),
}

/// An MF2 expression: an optional operand passed through an optional function
/// with options. At least one of operand / function is present (the parser
/// enforces a non-empty expression).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expression {
    /// The value being formatted: a variable reference or a literal. `None` for
    /// a standalone function expression (rare; kept for spec completeness).
    pub operand: Option<Operand>,
    /// The function applied (e.g. `:number`, `:string`), with its options.
    pub function: Option<FunctionRef>,
}

/// The value an [`Expression`] formats or selects on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// A variable reference (`$count`); resolved against the format arguments.
    Variable(String),
    /// A literal value (`|quoted|`, a number, or a bareword).
    Literal(String),
}

/// A function applied in an expression: its name (without the leading `:`) and
/// its `option=value` pairs (order-preserving for deterministic rendering).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionRef {
    /// The function name, e.g. `number`, `integer`, `string`.
    pub name: String,
    /// The options, as `(name, value)` pairs. A value is a literal or a
    /// `$variable` (kept as the raw token; resolved at format time).
    pub options: Vec<(String, OptionValue)>,
}

/// An option value: a literal string or a variable reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionValue {
    /// A literal option value (`maximumFractionDigits=2`).
    Literal(String),
    /// A variable option value (`minimumFractionDigits=$digits`).
    Variable(String),
}

/// A `.input` or `.local` declaration evaluated before the pattern/selectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declaration {
    /// `.input {$name :func ...}` - declares/annotates an external argument.
    Input {
        /// The declared variable name (without `$`).
        name: String,
        /// The annotating expression.
        expression: Expression,
    },
    /// `.local $name = {expr}` - a local bound to an expression's value.
    Local {
        /// The local variable name (without `$`).
        name: String,
        /// The bound expression.
        expression: Expression,
    },
}

/// A variant in a select message: the keys it matches and the pattern it yields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    /// One key per selector. A [`VariantKey::CatchAll`] (`*`) matches anything.
    pub keys: Vec<VariantKey>,
    /// The pattern rendered when this variant is selected.
    pub pattern: Pattern,
}

/// A variant key: a literal match or the catch-all `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariantKey {
    /// Matches when the selector resolves to this exact key (a plural category
    /// like `one`/`other`, or an exact literal).
    Literal(String),
    /// The fallback `*` - matches any selector value.
    CatchAll,
}
