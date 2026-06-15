//! A focused MessageFormat 2.0 parser (i18n-plan.md I18N-R1).
//!
//! Hand-rolled recursive descent over the MF2 syntax, building the
//! [`crate::model`]. It covers the message shapes Arlen's catalogs use:
//!
//! - SIMPLE messages: literal text with `{...}` placeholders, e.g.
//!   `Hello, {$name}!` or `{$count :number} items`;
//! - COMPLEX messages: `.input` / `.local` declarations, a quoted pattern
//!   `{{...}}`, and the `.match` selector with keyed variants (the plural /
//!   select form that makes translations correct).
//!
//! It is fail-closed: a malformed message returns a [`ParseError`] (never a
//! panic), so the caller can fall back to the key. Exotic spec corners (markup
//! `{#tag}`, reserved/private annotations) are not parsed yet - they return a
//! clear error rather than a wrong tree; the catalogs do not use them, and the
//! data model is spec-shaped so they slot in later without a catalog rewrite.

use crate::model::{
    Declaration, Expression, FunctionRef, Message, Operand, OptionValue, Part, Pattern, Variant,
    VariantKey,
};

/// A message that did not parse as MF2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// A human-readable reason (for logs; never shown to a user).
    pub message: String,
    /// The character offset where parsing failed.
    pub offset: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MF2 parse error at {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parse an MF2 message string into the [`Message`] model.
pub fn parse_message(input: &str) -> Result<Message, ParseError> {
    Parser::new(input).parse_message()
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
        }
    }

    fn err(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            offset: self.pos,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.pos + ahead).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\r' | '\n')) {
            self.pos += 1;
        }
    }

    /// Whether the remaining input starts with `kw`.
    fn looking_at(&self, kw: &str) -> bool {
        kw.chars()
            .enumerate()
            .all(|(i, c)| self.peek_at(i) == Some(c))
    }

    fn parse_message(&mut self) -> Result<Message, ParseError> {
        // A complex message begins (after optional ws) with a `.input`/`.local`
        // declaration or a `.match`. Everything else is a simple message whose
        // entire text is the pattern.
        let save = self.pos;
        self.skip_ws();
        let complex = self.looking_at(".input") || self.looking_at(".local") || self.looking_at(".match");
        self.pos = save;
        if complex {
            self.parse_complex()
        } else {
            let pattern = self.parse_simple_pattern()?;
            Ok(Message::Pattern {
                declarations: Vec::new(),
                pattern,
            })
        }
    }

    /// A simple message: the whole input is one pattern (text + `{...}`).
    fn parse_simple_pattern(&mut self) -> Result<Pattern, ParseError> {
        let pattern = self.parse_pattern_body(false)?;
        if self.pos < self.chars.len() {
            return Err(self.err("unexpected trailing input after the pattern"));
        }
        Ok(pattern)
    }

    /// Parse pattern parts (text + expressions). When `quoted`, stop at the
    /// closing `}}`; otherwise run to end of input.
    fn parse_pattern_body(&mut self, quoted: bool) -> Result<Pattern, ParseError> {
        let mut parts = Vec::new();
        let mut text = String::new();
        loop {
            match self.peek() {
                None => {
                    if quoted {
                        return Err(self.err("unterminated quoted pattern (expected `}}`)"));
                    }
                    break;
                }
                Some('}') if quoted && self.peek_at(1) == Some('}') => {
                    self.pos += 2; // consume `}}`
                    break;
                }
                Some('\\') => {
                    // Text escapes: \\ \{ \} \|
                    match self.peek_at(1) {
                        Some(c @ ('\\' | '{' | '}' | '|')) => {
                            text.push(c);
                            self.pos += 2;
                        }
                        _ => return Err(self.err("invalid escape in text (use \\\\ \\{ \\} \\|)")),
                    }
                }
                Some('{') => {
                    if !text.is_empty() {
                        parts.push(Part::Text(std::mem::take(&mut text)));
                    }
                    let expr = self.parse_expression()?;
                    parts.push(Part::Expression(expr));
                }
                Some(c) => {
                    text.push(c);
                    self.pos += 1;
                }
            }
        }
        if !text.is_empty() {
            parts.push(Part::Text(text));
        }
        Ok(parts)
    }

    /// Parse a `{ ... }` expression (the opening `{` is at the cursor).
    fn parse_expression(&mut self) -> Result<Expression, ParseError> {
        if self.bump() != Some('{') {
            return Err(self.err("expected `{`"));
        }
        self.skip_ws();
        let operand = match self.peek() {
            Some(':') => None, // a standalone function, no operand
            Some('}') => return Err(self.err("empty expression `{}`")),
            _ => Some(self.parse_operand()?),
        };
        self.skip_ws();
        let function = if self.peek() == Some(':') {
            Some(self.parse_function()?)
        } else {
            None
        };
        self.skip_ws();
        if self.bump() != Some('}') {
            return Err(self.err("expected `}` to close the expression"));
        }
        if operand.is_none() && function.is_none() {
            return Err(self.err("expression has neither an operand nor a function"));
        }
        Ok(Expression { operand, function })
    }

    /// Parse an operand: `$variable` or a literal.
    fn parse_operand(&mut self) -> Result<Operand, ParseError> {
        if self.peek() == Some('$') {
            self.pos += 1;
            let name = self.parse_name()?;
            Ok(Operand::Variable(name))
        } else {
            Ok(Operand::Literal(self.parse_literal()?))
        }
    }

    /// Parse a literal: `|quoted|` (with `\|` `\\` escapes) or an unquoted token
    /// (a number or a bareword - any run of non-syntax, non-whitespace chars).
    fn parse_literal(&mut self) -> Result<String, ParseError> {
        if self.peek() == Some('|') {
            self.pos += 1;
            let mut s = String::new();
            loop {
                match self.bump() {
                    None => return Err(self.err("unterminated quoted literal")),
                    Some('|') => return Ok(s),
                    Some('\\') => match self.bump() {
                        Some(c @ ('|' | '\\')) => s.push(c),
                        _ => return Err(self.err("invalid escape in quoted literal")),
                    },
                    Some(c) => s.push(c),
                }
            }
        } else {
            let s = self.parse_token();
            if s.is_empty() {
                return Err(self.err("expected a literal"));
            }
            Ok(s)
        }
    }

    /// Parse a `:function` annotation with its options.
    fn parse_function(&mut self) -> Result<FunctionRef, ParseError> {
        if self.bump() != Some(':') {
            return Err(self.err("expected `:` to start a function"));
        }
        let name = self.parse_name()?;
        let mut options = Vec::new();
        loop {
            // Options are whitespace-separated `name=value`. Stop at `}`.
            let save = self.pos;
            self.skip_ws();
            if matches!(self.peek(), None | Some('}')) {
                self.pos = save;
                break;
            }
            // Must be an option name; if not, rewind (lets the caller see `}`).
            if !Self::is_name_start(self.peek()) {
                self.pos = save;
                break;
            }
            let opt_name = self.parse_name()?;
            self.skip_ws();
            if self.bump() != Some('=') {
                return Err(self.err("expected `=` in a function option"));
            }
            self.skip_ws();
            let value = if self.peek() == Some('$') {
                self.pos += 1;
                OptionValue::Variable(self.parse_name()?)
            } else {
                OptionValue::Literal(self.parse_literal()?)
            };
            options.push((opt_name, value));
        }
        Ok(FunctionRef { name, options })
    }

    /// A complex message: declarations, then a `.match` or a `{{quoted pattern}}`.
    fn parse_complex(&mut self) -> Result<Message, ParseError> {
        let mut declarations = Vec::new();
        loop {
            self.skip_ws();
            if self.looking_at(".input") {
                self.pos += ".input".len();
                self.skip_ws();
                let expression = self.parse_expression()?;
                let name = match &expression.operand {
                    Some(Operand::Variable(v)) => v.clone(),
                    _ => return Err(self.err(".input must annotate a $variable")),
                };
                declarations.push(Declaration::Input { name, expression });
            } else if self.looking_at(".local") {
                self.pos += ".local".len();
                self.skip_ws();
                if self.bump() != Some('$') {
                    return Err(self.err(".local must declare a $variable"));
                }
                let name = self.parse_name()?;
                self.skip_ws();
                if self.bump() != Some('=') {
                    return Err(self.err("expected `=` in .local"));
                }
                self.skip_ws();
                let expression = self.parse_expression()?;
                declarations.push(Declaration::Local { name, expression });
            } else {
                break;
            }
        }
        self.skip_ws();
        if self.looking_at(".match") {
            self.pos += ".match".len();
            self.parse_match(declarations)
        } else if self.peek() == Some('{') && self.peek_at(1) == Some('{') {
            self.pos += 2; // consume `{{`
            let pattern = self.parse_pattern_body(true)?;
            self.skip_ws();
            if self.pos < self.chars.len() {
                return Err(self.err("unexpected trailing input after the quoted pattern"));
            }
            Ok(Message::Pattern {
                declarations,
                pattern,
            })
        } else {
            Err(self.err("a complex message needs a `.match` or a `{{...}}` quoted pattern"))
        }
    }

    /// Parse the `.match` selectors + variants (the `.match` keyword is consumed).
    fn parse_match(&mut self, declarations: Vec<Declaration>) -> Result<Message, ParseError> {
        let mut selectors = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                // A selector is an expression `{...}` or, as MF2 allows when the
                // variable was already annotated by a `.input`, a bare `$var`.
                Some('{') => selectors.push(self.parse_expression()?),
                Some('$') => {
                    self.pos += 1;
                    let name = self.parse_name()?;
                    selectors.push(Expression {
                        operand: Some(Operand::Variable(name)),
                        function: None,
                    });
                }
                _ => break,
            }
        }
        if selectors.is_empty() {
            return Err(self.err(".match needs at least one selector"));
        }
        let mut variants = Vec::new();
        loop {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                break;
            }
            // A variant: one or more keys, then a `{{quoted pattern}}`.
            let mut keys = Vec::new();
            loop {
                self.skip_ws();
                if self.peek() == Some('{') && self.peek_at(1) == Some('{') {
                    break;
                }
                if self.peek() == Some('*') {
                    self.pos += 1;
                    keys.push(VariantKey::CatchAll);
                } else {
                    let lit = self.parse_literal()?;
                    keys.push(VariantKey::Literal(lit));
                }
            }
            if keys.is_empty() {
                return Err(self.err("a variant needs at least one key"));
            }
            if self.peek() == Some('{') && self.peek_at(1) == Some('{') {
                self.pos += 2;
                let pattern = self.parse_pattern_body(true)?;
                variants.push(Variant { keys, pattern });
            } else {
                return Err(self.err("a variant needs a `{{...}}` pattern after its keys"));
            }
        }
        if variants.is_empty() {
            return Err(self.err(".match needs at least one variant"));
        }
        // Every variant must have one key per selector.
        if let Some(bad) = variants.iter().find(|v| v.keys.len() != selectors.len()) {
            return Err(ParseError {
                message: format!(
                    "a variant has {} keys but there are {} selectors",
                    bad.keys.len(),
                    selectors.len()
                ),
                offset: self.pos,
            });
        }
        Ok(Message::Select {
            declarations,
            selectors,
            variants,
        })
    }

    /// A name (identifier): a run of non-syntax, non-whitespace characters.
    fn parse_name(&mut self) -> Result<String, ParseError> {
        if !Self::is_name_start(self.peek()) {
            return Err(self.err("expected a name"));
        }
        Ok(self.parse_token())
    }

    /// A token: a run of characters that are not MF2 syntax or whitespace.
    fn parse_token(&mut self) -> String {
        let mut s = String::new();
        while let Some(c) = self.peek() {
            if Self::is_token_char(c) {
                s.push(c);
                self.pos += 1;
            } else {
                break;
            }
        }
        s
    }

    fn is_name_start(c: Option<char>) -> bool {
        matches!(c, Some(c) if Self::is_token_char(c))
    }

    /// A character that may appear in an unquoted name / literal token. Excludes
    /// the MF2 syntax characters and whitespace.
    fn is_token_char(c: char) -> bool {
        !c.is_whitespace() && !matches!(c, '{' | '}' | '|' | '=' | ':' | '$' | '*' | '\\' | '@' | '#')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(m: &Message) -> &Pattern {
        match m {
            Message::Pattern { pattern, .. } => pattern,
            _ => panic!("expected a pattern message"),
        }
    }

    #[test]
    fn plain_text_is_one_text_part() {
        let m = parse_message("Hello, world!").unwrap();
        assert_eq!(pattern(&m), &vec![Part::Text("Hello, world!".into())]);
    }

    #[test]
    fn a_variable_placeholder_splits_text_and_expression() {
        let m = parse_message("Hello, {$name}!").unwrap();
        let p = pattern(&m);
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], Part::Text("Hello, ".into()));
        assert_eq!(
            p[1],
            Part::Expression(Expression {
                operand: Some(Operand::Variable("name".into())),
                function: None,
            })
        );
        assert_eq!(p[2], Part::Text("!".into()));
    }

    #[test]
    fn a_function_with_options_parses() {
        let m = parse_message("{$count :number minimumFractionDigits=2}").unwrap();
        let p = pattern(&m);
        let Part::Expression(e) = &p[0] else {
            panic!("expected an expression");
        };
        assert_eq!(e.operand, Some(Operand::Variable("count".into())));
        let f = e.function.as_ref().unwrap();
        assert_eq!(f.name, "number");
        assert_eq!(f.options, vec![("minimumFractionDigits".into(), OptionValue::Literal("2".into()))]);
    }

    #[test]
    fn text_escapes_resolve() {
        let m = parse_message(r"a \{ b \} c \| d \\ e").unwrap();
        assert_eq!(pattern(&m), &vec![Part::Text(r"a { b } c | d \ e".into())]);
    }

    #[test]
    fn a_quoted_literal_operand_keeps_inner_text() {
        let m = parse_message("{|hello world| :string}").unwrap();
        let Part::Expression(e) = &pattern(&m)[0] else {
            panic!();
        };
        assert_eq!(e.operand, Some(Operand::Literal("hello world".into())));
    }

    #[test]
    fn a_complex_message_with_input_and_quoted_pattern() {
        let m = parse_message(".input {$n :integer}\n{{You have {$n} items}}").unwrap();
        let Message::Pattern { declarations, pattern } = &m else {
            panic!("expected a pattern message");
        };
        assert_eq!(declarations.len(), 1);
        assert!(matches!(&declarations[0], Declaration::Input { name, .. } if name == "n"));
        assert!(pattern.iter().any(|p| matches!(p, Part::Expression(e) if e.operand == Some(Operand::Variable("n".into())))));
    }

    #[test]
    fn a_match_message_parses_selectors_and_variants() {
        let src = ".input {$count :number}\n.match $count\none {{One item}}\n*   {{{$count} items}}";
        let m = parse_message(src).unwrap();
        let Message::Select { declarations, selectors, variants } = &m else {
            panic!("expected a select message");
        };
        assert_eq!(declarations.len(), 1);
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].operand, Some(Operand::Variable("count".into())));
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].keys, vec![VariantKey::Literal("one".into())]);
        assert_eq!(variants[1].keys, vec![VariantKey::CatchAll]);
    }

    #[test]
    fn a_local_declaration_parses() {
        let m = parse_message(".local $x = {$y :number}\n{{value {$x}}}").unwrap();
        let Message::Pattern { declarations, .. } = &m else {
            panic!();
        };
        assert!(matches!(&declarations[0], Declaration::Local { name, .. } if name == "x"));
    }

    #[test]
    fn malformed_messages_error_without_panicking() {
        assert!(parse_message("{$unclosed").is_err());
        assert!(parse_message("{}").is_err(), "empty expression");
        assert!(parse_message(".match\n* {{x}}").is_err(), "no selector");
        assert!(parse_message(".match {$a}\none two {{x}}").is_err(), "variant key count != selector count");
        assert!(parse_message("{|unterminated}").is_err());
        assert!(parse_message(r"bad \x escape").is_err());
        // A bare `.match` with no variants.
        assert!(parse_message(".match {$a}").is_err());
    }

    #[test]
    fn a_multi_selector_match_requires_matching_key_counts() {
        let src = ".match {$a} {$b}\none one {{both one}}\n* * {{fallback}}";
        let m = parse_message(src).unwrap();
        let Message::Select { selectors, variants, .. } = &m else {
            panic!();
        };
        assert_eq!(selectors.len(), 2);
        assert_eq!(variants[0].keys.len(), 2);
    }
}
