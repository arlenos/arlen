//! The MessageFormat 2.0 formatter (i18n-plan.md I18N-R1), in-house over ICU4X's
//! plural + decimal primitives.
//!
//! Given a parsed [`Message`], a locale, and the caller's arguments, it renders
//! the final string: substituting placeholders, formatting numbers in the
//! locale (ICU4X `DecimalFormatter`), and choosing the right variant of a select
//! message by plural category (ICU4X `PluralRules`) or exact match. This is the
//! piece the i18n-plan flags as swappable: when ICU4X ships its own MF2
//! formatter the catalogs and the [`crate::model`] stay; only this rendering
//! backend changes.
//!
//! Scope of the first cut (honest, not a wrong tree): the `:number`/`:integer`
//! and `:string` functions, plural + exact-literal selection, and `.input` /
//! `.local` declarations. A function it does not know is treated as a pass-through
//! (the operand renders as text) rather than failing - a translation should
//! never blow up, only render plainly.

use std::collections::BTreeMap;

use icu_decimal::input::Decimal;
use icu_decimal::DecimalFormatter;
use icu_locale_core::Locale;
use icu_plurals::{PluralCategory, PluralRules};

use crate::model::{
    Declaration, Expression, Message, Operand, Part, Pattern, Variant, VariantKey,
};

/// An argument value supplied to format a message.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgValue {
    /// An integer (the common plural operand).
    Integer(i64),
    /// A floating-point number.
    Float(f64),
    /// A text value.
    Text(String),
}

impl ArgValue {
    /// Render the value as plain text (the fallback when no locale-aware
    /// function applies).
    fn as_plain(&self) -> String {
        match self {
            ArgValue::Integer(n) => n.to_string(),
            ArgValue::Float(f) => f.to_string(),
            ArgValue::Text(s) => s.clone(),
        }
    }

    /// The value as a [`Decimal`] for locale-aware number formatting + plural
    /// selection, when it is numeric. A `Float` round-trips to a decimal so it
    /// too formats with the locale's grouping + decimal separator.
    fn as_decimal(&self) -> Option<Decimal> {
        match self {
            ArgValue::Integer(n) => Some(Decimal::from(*n)),
            ArgValue::Float(f) => {
                Decimal::try_from_f64(*f, fixed_decimal::FloatPrecision::RoundTrip).ok()
            }
            // A numeric-looking text value is accepted so a catalog can pass a
            // pre-formatted count; a non-numeric text is not a number.
            ArgValue::Text(s) => s.parse::<i64>().ok().map(Decimal::from),
        }
    }
}

/// The caller's named arguments.
pub type Args = BTreeMap<String, ArgValue>;

/// Format `message` for `locale` with `args`, returning the rendered string.
///
/// Infallible by design: a missing argument renders as the bare variable name in
/// braces (a visible, debuggable placeholder, never a panic), and an unknown
/// function passes its operand through as text.
pub fn format(message: &Message, locale: &Locale, args: &Args) -> String {
    match message {
        Message::Pattern {
            declarations,
            pattern,
        } => {
            let env = Env::build(declarations, args);
            render_pattern(pattern, locale, args, &env)
        }
        Message::Select {
            declarations,
            selectors,
            variants,
        } => {
            let env = Env::build(declarations, args);
            let variant = select_variant(selectors, variants, locale, args, &env);
            match variant {
                Some(v) => render_pattern(&v.pattern, locale, args, &env),
                None => String::new(),
            }
        }
    }
}

/// The declaration environment: which variables a `.input`/`.local` annotated
/// with a number function (so a selector on them uses plural rules), keyed by
/// name. The first cut tracks the numeric annotation; a `.local`'s bound value
/// resolves from the underlying argument at use.
struct Env {
    numeric: BTreeMap<String, ()>,
}

impl Env {
    fn build(declarations: &[Declaration], _args: &Args) -> Self {
        let mut numeric = BTreeMap::new();
        for d in declarations {
            let (name, expr) = match d {
                Declaration::Input { name, expression } => (name, expression),
                Declaration::Local { name, expression } => (name, expression),
            };
            if is_number_function(expr) {
                numeric.insert(name.clone(), ());
            }
        }
        Env { numeric }
    }

    fn is_numeric(&self, name: &str) -> bool {
        self.numeric.contains_key(name)
    }
}

/// Whether an expression's function is a number formatter (`:number`/`:integer`).
fn is_number_function(expr: &Expression) -> bool {
    expr.function
        .as_ref()
        .is_some_and(|f| matches!(f.name.as_str(), "number" | "integer"))
}

/// Render a pattern (text + expressions) to a string.
fn render_pattern(pattern: &Pattern, locale: &Locale, args: &Args, env: &Env) -> String {
    let mut out = String::new();
    for part in pattern {
        match part {
            Part::Text(t) => out.push_str(t),
            Part::Expression(e) => out.push_str(&format_expression(e, locale, args, env)),
        }
    }
    out
}

/// Format one expression to its placeholder text.
fn format_expression(expr: &Expression, locale: &Locale, args: &Args, _env: &Env) -> String {
    let value = match &expr.operand {
        Some(Operand::Variable(name)) => match args.get(name) {
            Some(v) => v.clone(),
            // A missing argument is shown as a visible debuggable placeholder.
            None => return format!("{{${name}}}"),
        },
        Some(Operand::Literal(s)) => ArgValue::Text(s.clone()),
        None => return String::new(),
    };

    let func = expr.function.as_ref().map(|f| f.name.as_str());
    match func {
        Some("number") | Some("integer") => match value.as_decimal() {
            Some(dec) => match DecimalFormatter::try_new(locale.into(), Default::default()) {
                Ok(fmt) => fmt.format(&dec).to_string(),
                // No data for the locale: fall back to the plain rendering.
                Err(_) => value.as_plain(),
            },
            None => value.as_plain(),
        },
        // `:string`, no function, or an unknown function: render as plain text.
        _ => value.as_plain(),
    }
}

/// Choose the matching variant of a select message.
///
/// MF2 selection, focused cut: a selector resolves to a key - a plural category
/// (`one`/`other`/...) for a numeric selector, otherwise the exact value text. A
/// variant matches when every key matches its selector (a `*` catch-all always
/// matches, an exact category/text matches its kind). Among matching variants
/// the most specific wins (fewest catch-alls), then source order; if none match,
/// the first all-catch-all variant, else the first variant.
fn select_variant<'v>(
    selectors: &[Expression],
    variants: &'v [Variant],
    locale: &Locale,
    args: &Args,
    env: &Env,
) -> Option<&'v Variant> {
    let keys: Vec<SelectorKey> = selectors
        .iter()
        .map(|s| resolve_selector(s, locale, args, env))
        .collect();

    let mut best: Option<(&Variant, u32)> = None;
    for variant in variants {
        if variant.keys.len() != keys.len() {
            continue;
        }
        // Score each key by match STRENGTH: an exact-value match (`1`) beats a
        // plural-category match (`one`) beats the catch-all `*`, regardless of
        // source order. A variant matches only if every key matches; its score
        // is the summed strength, so the most-specific matching variant wins.
        let mut score = 0u32;
        let mut matched = true;
        for (vk, sel) in variant.keys.iter().zip(&keys) {
            match vk {
                VariantKey::CatchAll => {} // strength 0
                VariantKey::Literal(lit) => match sel.strength(lit) {
                    Some(s) => score += s as u32,
                    None => {
                        matched = false;
                        break;
                    }
                },
            }
        }
        if matched && best.is_none_or(|(_, best_score)| score > best_score) {
            best = Some((variant, score));
        }
    }
    best.map(|(v, _)| v)
        .or_else(|| variants.iter().find(|v| v.keys.iter().all(|k| matches!(k, VariantKey::CatchAll))))
        .or_else(|| variants.first())
}

/// What a selector resolved to: a numeric value (which matches both its exact
/// digits and its plural category) or an exact text.
enum SelectorKey {
    /// A numeric selector: `exact` is the value's digits (matches an exact key
    /// like `0`), `category` is its CLDR plural category (matches `one`/`*`).
    Plural {
        /// The exact value text (e.g. `"0"`).
        exact: String,
        /// The CLDR plural category for the value in the locale.
        category: PluralCategory,
    },
    /// A non-numeric selector: matches its exact text.
    Exact(String),
}

impl SelectorKey {
    /// The match STRENGTH of a variant's literal key against this selector, or
    /// `None` if it does not match. An exact-value match scores `2`, a plural-
    /// category match `1`, so [`select_variant`] prefers the exact key over the
    /// category key regardless of their source order (the MF2 specificity rule).
    fn strength(&self, lit: &str) -> Option<u8> {
        match self {
            SelectorKey::Plural { exact, category } => {
                if lit == exact {
                    Some(2)
                } else if plural_category_key(*category) == lit {
                    Some(1)
                } else {
                    None
                }
            }
            SelectorKey::Exact(v) => (v == lit).then_some(2),
        }
    }
}

/// Resolve a selector expression to its [`SelectorKey`].
fn resolve_selector(expr: &Expression, locale: &Locale, args: &Args, env: &Env) -> SelectorKey {
    let value = match &expr.operand {
        Some(Operand::Variable(name)) => args.get(name).cloned(),
        Some(Operand::Literal(s)) => Some(ArgValue::Text(s.clone())),
        None => None,
    };
    let Some(value) = value else {
        return SelectorKey::Exact(String::new());
    };

    // Numeric selection when the selector annotates a number function OR the
    // variable was `.input`/`.local`-annotated as numeric OR the value is numeric.
    let numeric_var = matches!(&expr.operand, Some(Operand::Variable(n)) if env.is_numeric(n));
    let numeric = is_number_function(expr) || numeric_var;
    if numeric {
        if let Some(dec) = value.as_decimal() {
            if let Ok(pr) = PluralRules::try_new(locale.into(), Default::default()) {
                return SelectorKey::Plural {
                    exact: value.as_plain(),
                    category: pr.category_for(&dec),
                };
            }
        }
    }
    // An exact numeric literal key (e.g. `0`) should still match a numeric value.
    SelectorKey::Exact(value.as_plain())
}

/// The MF2 / CLDR key name for a plural category.
fn plural_category_key(cat: PluralCategory) -> &'static str {
    match cat {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_message;

    fn en() -> Locale {
        "en".parse().unwrap()
    }

    fn args(pairs: &[(&str, ArgValue)]) -> Args {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn substitutes_a_variable() {
        let m = parse_message("Hello, {$name}!").unwrap();
        let out = format(&m, &en(), &args(&[("name", ArgValue::Text("Tim".into()))]));
        assert_eq!(out, "Hello, Tim!");
    }

    #[test]
    fn a_missing_argument_is_a_visible_placeholder_not_a_panic() {
        let m = parse_message("Hello, {$name}!").unwrap();
        assert_eq!(format(&m, &en(), &args(&[])), "Hello, {$name}!");
    }

    #[test]
    fn formats_a_number_in_the_locale() {
        let m = parse_message("{$n :number}").unwrap();
        // en groups with commas.
        assert_eq!(format(&m, &en(), &args(&[("n", ArgValue::Integer(1234567))])), "1,234,567");
        let de: Locale = "de".parse().unwrap();
        // de groups with dots.
        assert_eq!(format(&m, &de, &args(&[("n", ArgValue::Integer(1234567))])), "1.234.567");
        // A float formats with the locale's grouping + decimal separator too.
        assert_eq!(format(&m, &de, &args(&[("n", ArgValue::Float(1234.5))])), "1.234,5");
        assert_eq!(format(&m, &en(), &args(&[("n", ArgValue::Float(1234.5))])), "1,234.5");
    }

    #[test]
    fn plural_selection_picks_one_vs_other() {
        let src = ".input {$count :number}\n.match $count\none {{{$count} item}}\n* {{{$count} items}}";
        let m = parse_message(src).unwrap();
        assert_eq!(format(&m, &en(), &args(&[("count", ArgValue::Integer(1))])), "1 item");
        assert_eq!(format(&m, &en(), &args(&[("count", ArgValue::Integer(5))])), "5 items");
    }

    #[test]
    fn an_exact_numeric_key_matches_before_the_category() {
        let src = ".input {$count :number}\n.match $count\n0 {{no items}}\none {{one item}}\n* {{{$count} items}}";
        let m = parse_message(src).unwrap();
        assert_eq!(format(&m, &en(), &args(&[("count", ArgValue::Integer(0))])), "no items");
        assert_eq!(format(&m, &en(), &args(&[("count", ArgValue::Integer(1))])), "one item");
        assert_eq!(format(&m, &en(), &args(&[("count", ArgValue::Integer(3))])), "3 items");
    }

    #[test]
    fn an_exact_key_beats_the_category_even_when_listed_after_it() {
        // Specificity, not source order, decides: `1` (exact) must win over
        // `one` (category) for count=1 even though `one` is listed first.
        let src = ".match {$n :number}\none {{category}}\n1 {{exact}}\n* {{star}}";
        let m = parse_message(src).unwrap();
        assert_eq!(format(&m, &en(), &args(&[("n", ArgValue::Integer(1))])), "exact");
        // count=2: no exact key, category `one` does not match -> `*`.
        assert_eq!(format(&m, &en(), &args(&[("n", ArgValue::Integer(2))])), "star");
    }

    #[test]
    fn an_exact_string_selector_matches() {
        let src = ".match {$kind}\nfile {{a file}}\n* {{something}}";
        let m = parse_message(src).unwrap();
        assert_eq!(format(&m, &en(), &args(&[("kind", ArgValue::Text("file".into()))])), "a file");
        assert_eq!(format(&m, &en(), &args(&[("kind", ArgValue::Text("dir".into()))])), "something");
    }

    #[test]
    fn an_unknown_function_passes_the_operand_through() {
        let m = parse_message("{$x :weird}").unwrap();
        assert_eq!(format(&m, &en(), &args(&[("x", ArgValue::Text("y".into()))])), "y");
    }
}
