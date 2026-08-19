// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! When the two halves of a `multipart/alternative` say different things.
//!
//! `mail-app.md` section 3 ends on a rule that is easy to read past and is the
//! whole of this module: **when the text and HTML parts disagree, that is
//! information about the message and should be surfaced, not silently resolved
//! in favour of one part.**
//!
//! Every mail client resolves it silently. They pick the HTML, or they pick the
//! text, and the sender's two versions are never compared - which is precisely
//! what makes the disagreement useful to somebody sending mail in bad faith: a
//! plain-text part that reads innocently for whatever scans it, and an HTML part
//! that reads differently for the person. It is the parser-differential shape of
//! section 2 moved up a layer, from two programs disagreeing about one part to
//! one message carrying two accounts of itself.
//!
//! This does not decide which to show. It reports what one says and the other
//! does not, and the surface says so. A client that resolved it would be making
//! the sender's choice for them.
//!
//! **The comparison is deliberately crude, and its crudeness is stated rather
//! than hidden.** Tags are stripped, entities are not expanded, whitespace is
//! collapsed, and the comparison is over words. That will call some innocent
//! messages different - a footer only in the HTML, a link written out in the
//! text - and it will not catch a difference expressed only in styling, which
//! is the one this cannot see at all. What it does catch is a sentence present
//! in one half and absent from the other, which is the shape that matters.

use std::collections::BTreeSet;

/// What the two halves of an alternative say about each other.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Divergence {
    /// Words the plain-text half has and the HTML half does not.
    pub only_in_text: Vec<String>,
    /// Words the HTML half has and the plain-text half does not.
    pub only_in_html: Vec<String>,
}

impl Divergence {
    /// Whether the two halves agree, as far as this can see.
    #[must_use]
    pub fn agree(&self) -> bool {
        self.only_in_text.is_empty() && self.only_in_html.is_empty()
    }

    /// What to tell a reader, or `None` when the halves agree.
    ///
    /// Phrased as an observation rather than an accusation: a mismatch is
    /// usually a mail client's footer, occasionally something worse, and the
    /// message cannot tell which. Naming a few of the words lets the reader
    /// judge, which is the entire point of not resolving it for them.
    #[must_use]
    pub fn notice(&self) -> Option<String> {
        if self.agree() {
            return None;
        }
        let sample = |words: &[String]| {
            words.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
        };
        Some(match (self.only_in_text.is_empty(), self.only_in_html.is_empty()) {
            (false, false) => format!(
                "the plain-text and formatted versions of this message differ: only in the text ({}), only in the formatting ({})",
                sample(&self.only_in_text),
                sample(&self.only_in_html)
            ),
            (false, true) => format!(
                "the plain-text version of this message says things the formatted one does not ({})",
                sample(&self.only_in_text)
            ),
            _ => format!(
                "the formatted version of this message says things the plain-text one does not ({})",
                sample(&self.only_in_html)
            ),
        })
    }
}

/// Compare the plain-text and HTML halves of a `multipart/alternative`.
#[must_use]
pub fn compare(text: &str, html: &str) -> Divergence {
    let t = words(text);
    let h = words(&visible_text(html));
    Divergence {
        only_in_text: t.difference(&h).cloned().collect(),
        only_in_html: h.difference(&t).cloned().collect(),
    }
}

/// The words of a string, lowercased, punctuation dropped.
fn words(s: &str) -> BTreeSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The text a reader would see, with the markup taken out.
///
/// A tag stripper, not a parser: everything between `<` and the next `>` goes,
/// and the contents of `script` and `style` go with it. Those two carry text
/// that is never shown, and counting it as visible would report a difference on
/// every styled message ever sent.
fn visible_text(html: &str) -> String {
    // Owned through the loop rather than reborrowed: an earlier cut of this
    // used `Box::leak` to get a `&str` back out of each pass, which leaks a
    // copy of the message body every time a mail is opened.
    let mut rest = html.to_string();
    for tag in ["script", "style"] {
        let mut cleaned = String::with_capacity(rest.len());
        let mut from = 0usize;
        while let Some(start) = find_ci(&rest[from..], &format!("<{tag}")) {
            let start = from + start;
            cleaned.push_str(&rest[from..start]);
            match find_ci(&rest[start..], &format!("</{tag}")) {
                Some(end) => {
                    let close = start + end;
                    match rest[close..].find('>') {
                        Some(gt) => from = close + gt + 1,
                        None => {
                            from = rest.len();
                            break;
                        }
                    }
                }
                // An unclosed script runs to the end of the document, which is
                // how a browser reads it too.
                None => {
                    from = rest.len();
                    break;
                }
            }
        }
        cleaned.push_str(&rest[from.min(rest.len())..]);
        rest = cleaned;
    }

    let mut out = String::with_capacity(rest.len());
    let mut in_tag = false;
    for c in rest.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Case-insensitive `find`, since HTML tag names are not case-sensitive.
fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack.to_lowercase().find(&needle.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_halves_that_say_the_same_thing_agree() {
        let d = compare("Hello there, meeting at four.", "<p>Hello there, meeting at four.</p>");
        assert!(d.agree(), "got {d:?}");
        assert_eq!(d.notice(), None);
    }

    #[test]
    fn markup_alone_is_not_a_difference() {
        let d = compare(
            "Please review the report.",
            "<div class=\"x\"><b>Please</b> <i>review</i> the <a href=\"#\">report</a>.</div>",
        );
        assert!(d.agree(), "got {d:?}");
    }

    #[test]
    fn a_sentence_only_in_the_formatted_half_is_reported() {
        // The case this exists for: one account of the message for whatever
        // reads the text, another for the person.
        let d = compare(
            "Your invoice is attached.",
            "<p>Your invoice is attached.</p><p>Wire the payment to account 12345 today.</p>",
        );
        assert!(!d.agree());
        assert!(d.only_in_html.contains(&"wire".to_string()), "got {d:?}");
        let notice = d.notice().expect("a divergence has something to say");
        assert!(notice.contains("formatted version"), "got {notice}");
    }

    #[test]
    fn a_sentence_only_in_the_plain_half_is_reported_too() {
        // The HTML half is a strict subset here, so this exercises the
        // one-sided notice rather than the both-sided one.
        let d = compare("Thanks. Cancel the order immediately.", "<p>Thanks.</p>");
        assert!(d.only_in_text.contains(&"cancel".to_string()), "got {d:?}");
        assert!(d.notice().expect("says something").contains("plain-text version"));
    }

    #[test]
    fn text_inside_script_and_style_is_not_counted_as_something_the_reader_sees() {
        // Otherwise every styled message in the world reports a divergence, and
        // a warning that fires on everything is a warning nobody reads.
        let d = compare(
            "Hello.",
            "<style>.a { colour: red }</style><script>var secret = 1;</script><p>Hello.</p>",
        );
        assert!(d.agree(), "got {d:?}");
    }

    #[test]
    fn an_unclosed_script_swallows_the_rest_the_way_a_browser_would() {
        let d = compare("Hello.", "<p>Hello.</p><script>never closed");
        assert!(d.agree(), "got {d:?}");
    }

    #[test]
    fn the_notice_names_both_directions_when_both_differ() {
        let d = compare("Only here alpha", "<p>Only there beta</p>");
        let notice = d.notice().expect("says something");
        assert!(notice.contains("only in the text"), "got {notice}");
        assert!(notice.contains("only in the formatting"), "got {notice}");
    }
}
