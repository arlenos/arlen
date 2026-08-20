// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Our stated rule for the part of MIME the standard left ambiguous.
//!
//! `mail-app.md` section 2 is built on a measured finding rather than folklore:
//! Andarzian, Meyers and Poll's differential fuzzing of MIME parsers (LangSec at
//! IEEE S&P Workshops, 2025) found 448 parser-differential cases and fifteen
//! distinct root causes, three of them exploitable to walk past a virus or spam
//! filter. Two are worth naming because they are the ones this file answers:
//!
//! - A message with **two `Content-Transfer-Encoding` headers**: Evolution
//!   decodes on the first, SpamAssassin on the second, ClamAV decodes anyway,
//!   Postfix does not decode at all.
//! - A message with **two `Content-Type` headers**: Evolution takes the first,
//!   ClamAV and SpamAssassin take the second.
//!
//! Both were weaponised in the paper so that a spam-test string renders in the
//! mail client while the scanner does not flag it. The attack does not need a
//! bug in either program: it needs them to disagree, and the standard never
//! resolved which is right.
//!
//! **So the rule is written down here rather than inherited by accident.** A
//! parser with no stated rule still has one, and that is how this class of bug
//! is born.
//!
//! # The rule
//!
//! For `Content-Type` and `Content-Transfer-Encoding`, where a part carries more
//! than one:
//!
//! 1. **No header**: the standard's own default applies (`text/plain`,
//!    `7bit`). Not an error; most mail says nothing and means exactly that.
//! 2. **One header**: it is the answer.
//! 3. **Several that agree**, ignoring case and surrounding space: they are one
//!    answer, and it is that answer. Duplication alone is not an attack.
//! 4. **Several that disagree**: THE PART IS UNDECIDABLE. No value is chosen.
//!
//! Rule 4 is the whole point and it is deliberately not "first wins" or "last
//! wins". Either of those is a position in the disagreement that makes us the
//! program a scanner can be played off against. Refusing to choose removes the
//! differential: a part we cannot read unambiguously is one we will not render,
//! and the reader is told the message is malformed rather than shown one of the
//! two readings as though it were the message.

pub mod alternative;
pub mod exfiltration;
pub mod sync;
pub mod message;

use std::collections::BTreeSet;

/// What the standard says when a part names nothing.
pub const DEFAULT_CONTENT_TYPE: &str = "text/plain";
/// See [`DEFAULT_CONTENT_TYPE`].
pub const DEFAULT_ENCODING: &str = "7bit";

/// The answer for one header on one part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decided {
    /// The part said nothing, so the standard's default stands.
    Default(&'static str),
    /// The part said this, once or several times in agreement.
    Stated(String),
    /// The part said several different things, so there is no answer.
    ///
    /// Carries what it said, in the order it said it, because a reader being
    /// told their message is malformed deserves to see what made it so - and
    /// because this is the shape that gets reported as an attack.
    Ambiguous(Vec<String>),
}

impl Decided {
    /// The value, when there is one.
    ///
    /// `None` for [`Decided::Ambiguous`], which is the point: a caller that
    /// wants a value has to face the case where there is not one, rather than
    /// receiving a plausible guess.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::Default(v) => Some(v),
            Self::Stated(v) => Some(v),
            Self::Ambiguous(_) => None,
        }
    }

    /// Whether this part can be read at all.
    #[must_use]
    pub fn is_decidable(&self) -> bool {
        !matches!(self, Self::Ambiguous(_))
    }
}

/// Decide one header from every value a part carried for it.
///
/// `default` is the standard's value for that header when the part is silent.
/// Comparison for agreement is case-insensitive and ignores surrounding space,
/// because `TEXT/PLAIN` and `text/plain ` are the same statement written twice,
/// not two statements.
#[must_use]
pub fn decide(values: &[String], default: &'static str) -> Decided {
    let stated: Vec<String> = values
        .iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect();
    match stated.len() {
        0 => Decided::Default(default),
        1 => Decided::Stated(stated[0].clone()),
        _ => {
            let distinct: BTreeSet<String> = stated.iter().map(|v| v.to_lowercase()).collect();
            if distinct.len() == 1 {
                Decided::Stated(stated[0].clone())
            } else {
                Decided::Ambiguous(stated)
            }
        }
    }
}

/// What a part says about itself, once the ambiguous cases are faced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartHeaders {
    /// The `Content-Type`.
    pub content_type: Decided,
    /// The `Content-Transfer-Encoding`.
    pub encoding: Decided,
}

impl PartHeaders {
    /// Read a part's two ambiguous headers from every value it carried.
    #[must_use]
    pub fn decide(content_types: &[String], encodings: &[String]) -> Self {
        Self {
            content_type: decide(content_types, DEFAULT_CONTENT_TYPE),
            encoding: decide(encodings, DEFAULT_ENCODING),
        }
    }

    /// Whether this part can be rendered at all.
    ///
    /// Either header being undecidable is enough to stop: a part whose type is
    /// clear but whose decoding is not is a part we would have to guess the
    /// bytes of, and a part whose decoding is clear but whose type is not is one
    /// we would have to guess how to show.
    #[must_use]
    pub fn is_renderable(&self) -> bool {
        self.content_type.is_decidable() && self.encoding.is_decidable()
    }

    /// Why this part will not be rendered, for a surface to say out loud.
    ///
    /// `None` when it will be. The sentence names the header and what the
    /// message claimed, because "this message is malformed" with no detail is
    /// indistinguishable from the client being broken.
    #[must_use]
    pub fn refusal(&self) -> Option<String> {
        let (header, values) = match (&self.content_type, &self.encoding) {
            (Decided::Ambiguous(v), _) => ("Content-Type", v),
            (_, Decided::Ambiguous(v)) => ("Content-Transfer-Encoding", v),
            _ => return None,
        };
        Some(format!(
            "this message gives {} more than once and they disagree ({}), so how to read it is not defined",
            header,
            values.join(", ")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_part_that_says_nothing_gets_the_standards_own_answer() {
        let p = PartHeaders::decide(&[], &[]);
        assert_eq!(p.content_type.value(), Some("text/plain"));
        assert_eq!(p.encoding.value(), Some("7bit"));
        assert!(p.is_renderable(), "silence is the ordinary case, not a fault");
    }

    #[test]
    fn a_part_that_says_it_once_is_taken_at_its_word() {
        let p = PartHeaders::decide(&v(&["text/html"]), &v(&["base64"]));
        assert_eq!(p.content_type.value(), Some("text/html"));
        assert_eq!(p.encoding.value(), Some("base64"));
    }

    #[test]
    fn saying_the_same_thing_twice_is_saying_it_once() {
        // Duplication alone is not an attack, and refusing it would reject mail
        // that is merely clumsy.
        let p = PartHeaders::decide(&v(&["text/plain", "TEXT/PLAIN "]), &v(&["base64", "base64"]));
        assert!(p.is_renderable());
        assert_eq!(p.content_type.value(), Some("text/plain"));
    }

    #[test]
    fn two_content_types_that_disagree_have_no_answer() {
        // The paper's case: Evolution takes the first, ClamAV and SpamAssassin
        // the second, and the gap between them is where the payload lives.
        let p = PartHeaders::decide(&v(&["text/plain", "text/html"]), &[]);
        assert_eq!(p.content_type.value(), None);
        assert!(!p.is_renderable());
    }

    #[test]
    fn two_encodings_that_disagree_have_no_answer() {
        // The other one: four programs, four behaviours, no standard to appeal
        // to. Being a fifth opinion is what this refuses to be.
        let p = PartHeaders::decide(&[], &v(&["base64", "quoted-printable"]));
        assert_eq!(p.encoding.value(), None);
        assert!(!p.is_renderable());
    }

    #[test]
    fn a_refused_part_says_which_header_and_what_it_claimed() {
        let p = PartHeaders::decide(&v(&["text/plain", "text/html"]), &[]);
        let why = p.refusal().expect("a refused part has a reason");
        assert!(why.contains("Content-Type"), "got {why}");
        assert!(why.contains("text/plain") && why.contains("text/html"), "got {why}");
    }

    #[test]
    fn a_readable_part_has_nothing_to_explain() {
        assert_eq!(PartHeaders::decide(&v(&["text/plain"]), &[]).refusal(), None);
    }

    #[test]
    fn an_empty_header_is_silence_rather_than_a_statement() {
        // A header present but blank says nothing, and pairing it with a real
        // one must not read as a disagreement.
        let p = PartHeaders::decide(&v(&["", "  ", "text/html"]), &[]);
        assert_eq!(p.content_type.value(), Some("text/html"));
    }
}
