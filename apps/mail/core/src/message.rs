// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! One message file, read into the facts this crate already knows how to judge.
//!
//! The rest of this crate decides what to do when MIME is ambiguous; nothing in
//! it could read a message. This is the layer that hands it one - and the reason
//! it takes a parser rather than splitting on `\r\n\r\n` itself is section 2: a
//! MIME parser written to avoid a dependency is a parser with its own
//! undocumented differentials, which is the exact thing the decisions above are
//! there to stop being accidental.
//!
//! **WHAT THIS DELIBERATELY DOES NOT DO.** It does not touch the HTML part
//! beyond noticing that one exists. Section 3 of the plan is not a hardening
//! preference: a Tauri app on Linux gets no WebKitGTK sandbox, `wry` never calls
//! `webkit_web_context_set_sandbox_enabled`, so untrusted mail HTML must not
//! reach the app's webview at all. Anything that renders it needs process
//! isolation of its own and that is not built yet.

use mail_parser::MimeHeaders;

use crate::{alternative, exfiltration, PartHeaders};

/// What a message file turned out to hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The sender as written, unparsed and unverified - a display name is
    /// whatever the sender typed, and treating it as identity is how the oldest
    /// trick in mail still works.
    pub from: Option<String>,
    /// The subject as written.
    pub subject: Option<String>,
    /// The date line as written.
    pub date: Option<String>,
    /// The `text/plain` body, when the message has one.
    pub text: Option<String>,
    /// Whether an HTML part exists. NOT its content: see the module note.
    pub has_html: bool,
    /// What the text and HTML parts say differently, when both exist.
    pub divergence: Option<String>,
    /// Why this message was refused, when the ambiguity rules refuse it.
    pub refusal: Option<String>,
    /// Headers that are themselves a way out of the machine.
    pub channels: Vec<String>,
    /// What the message CARRIES, named and measured, never opened.
    ///
    /// Same principle as `has_html`: say what is there without acting on it. A
    /// message with three files attached and one with none looked identical in
    /// the window until 21 August, which is a fact about somebody's mail that the
    /// surface simply did not mention. Nothing is extracted and nothing is
    /// written to disk by reading this.
    pub attachments: Vec<Attachment>,
}

/// One part the message carries, as the message describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// The filename as written by the sender, or `None` when it named none.
    ///
    /// UNVERIFIED and not a path: a sender chooses this string, and the two
    /// oldest tricks with it are a traversal (`../../.ssh/authorized_keys`) and a
    /// second extension (`invoice.pdf.exe`). Nothing here opens or saves the
    /// part, so neither trick has a target yet - but whoever adds a save button
    /// takes the name as a suggestion and not as a destination.
    pub name: Option<String>,
    /// The media type the message claims, as written.
    pub media_type: Option<String>,
    /// How many bytes the part decodes to.
    pub bytes: usize,
}

/// Read one message.
///
/// `Err` only when nothing could be parsed at all. A message that parses but is
/// ambiguous comes back as a `Message` carrying its own refusal, because "this
/// file is not a message" and "this message says two contradictory things about
/// itself" are different answers and only one of them is about the file.
pub fn read(raw: &[u8]) -> Result<Message, String> {
    let parsed = mail_parser::MessageParser::default()
        .parse(raw)
        .ok_or_else(|| "this file could not be read as a message".to_string())?;

    // EVERY value, not the first. The decision rules exist because a message can
    // carry two `Content-Type` headers that disagree, and a reader that keeps
    // only one has already made the choice those rules are there to refuse.
    let mut content_types: Vec<String> = Vec::new();
    let mut encodings: Vec<String> = Vec::new();
    let mut raw_headers: Vec<(String, String)> = Vec::new();
    for header in parsed.headers() {
        let name = header.name().to_string();
        let value = header_text(raw, header);
        if name.eq_ignore_ascii_case("content-type") {
            content_types.push(value.clone());
        } else if name.eq_ignore_ascii_case("content-transfer-encoding") {
            encodings.push(value.clone());
        }
        raw_headers.push((name, value));
    }

    let headers = PartHeaders::decide(&content_types, &encodings);
    let text = parsed.body_text(0).map(|t| t.into_owned());
    // AN HTML PART, NOT A RENDERING OF THE TEXT ONE. `body_html(0)` answers
    // Some for a plain-text message - the parser converts the text part for
    // convenience - so asking it whether the message has HTML makes every plain
    // message claim an HTML part it does not have. That is a claim about
    // somebody's mail, and the surface would repeat it. Ask the part what it
    // says it is instead.
    let html = parsed
        .html_body
        .first()
        .and_then(|id| parsed.part(*id))
        .filter(|part| {
            part.content_type()
                .and_then(|c| c.subtype())
                .is_some_and(|sub| sub.eq_ignore_ascii_case("html"))
        })
        .and_then(|_| parsed.body_html(0).map(|h| h.into_owned()));
    let divergence = match (&text, &html) {
        (Some(t), Some(h)) => alternative::compare(t, h).notice(),
        _ => None,
    };

    // Named and measured, never opened: `contents()` is the decoded bytes and the
    // only thing taken from them is the length.
    let attachments: Vec<Attachment> = parsed
        .attachments()
        .map(|part| Attachment {
            name: part.attachment_name().map(str::to_string),
            media_type: part.content_type().map(|c| match c.subtype() {
                Some(sub) => format!("{}/{}", c.ctype(), sub),
                None => c.ctype().to_string(),
            }),
            bytes: part.contents().len(),
        })
        .collect();

    Ok(Message {
        from: parsed.from().and_then(|a| a.first()).and_then(|a| a.address().map(str::to_string)),
        subject: parsed.subject().map(str::to_string),
        date: parsed.date().map(|d| d.to_rfc3339()),
        text,
        has_html: html.is_some(),
        divergence,
        refusal: headers.refusal(),
        channels: exfiltration::header_channels(&raw_headers)
            .into_iter()
            .map(|f| f.header)
            .collect(),
        attachments,
    })
}

/// One header's value as written, from the raw bytes it was found at.
///
/// The parser hands back an offset rather than a string for headers it has no
/// typed form for, and the ambiguity rules compare values verbatim - a
/// normalised value would make two headers that differ look like one that does
/// not.
fn header_text(raw: &[u8], header: &mail_parser::Header) -> String {
    let start = header.offset_start as usize;
    let end = header.offset_end as usize;
    raw.get(start..end)
        .map(|b| String::from_utf8_lossy(b).trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_message_says_what_it_carries_without_opening_it() {
        let raw = b"From: a@example.org\r\n\
Subject: with a file\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=b1\r\n\
\r\n\
--b1\r\n\
Content-Type: text/plain\r\n\
\r\n\
see attached\r\n\
--b1\r\n\
Content-Type: application/pdf; name=\"invoice.pdf\"\r\n\
Content-Disposition: attachment; filename=\"invoice.pdf\"\r\n\
\r\n\
%PDF-1.4 pretend\r\n\
--b1--\r\n";
        let m = read(raw).unwrap();
        assert_eq!(m.attachments.len(), 1);
        assert_eq!(m.attachments[0].name.as_deref(), Some("invoice.pdf"));
        assert_eq!(m.attachments[0].media_type.as_deref(), Some("application/pdf"));
        assert!(m.attachments[0].bytes > 0, "measured, not guessed");
        assert_eq!(m.text.as_deref().map(str::trim), Some("see attached"));
    }

    #[test]
    fn a_message_with_nothing_attached_says_so_by_carrying_none() {
        let m = read(b"From: a@example.org\r\nSubject: plain\r\n\r\njust text\r\n").unwrap();
        assert!(m.attachments.is_empty());
    }

    #[test]
    fn a_sender_chosen_filename_is_kept_verbatim_and_is_not_a_path() {
        // The name is a claim, not a destination. Kept as written so a surface can
        // show what the sender actually called it; whoever adds a save button
        // treats this as a suggestion.
        let raw = b"From: a@example.org\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=b1\r\n\
\r\n\
--b1\r\n\
Content-Type: application/octet-stream\r\n\
Content-Disposition: attachment; filename=\"../../.ssh/authorized_keys\"\r\n\
\r\n\
ssh-rsa AAAA\r\n\
--b1--\r\n";
        let m = read(raw).unwrap();
        assert_eq!(m.attachments[0].name.as_deref(), Some("../../.ssh/authorized_keys"));
    }
    use super::*;

    fn msg(extra: &str, body: &str) -> Vec<u8> {
        format!(
            "From: someone@example.com\r\nSubject: A subject\r\n\
             Date: Tue, 19 Aug 2026 09:00:00 +0000\r\n{extra}\r\n{body}"
        )
        .into_bytes()
    }

    #[test]
    fn a_plain_message_comes_back_with_its_own_words() {
        let m = read(&msg("Content-Type: text/plain\r\n", "Hello, this is the body.")).unwrap();
        assert_eq!(m.subject.as_deref(), Some("A subject"));
        assert_eq!(m.from.as_deref(), Some("someone@example.com"));
        assert!(m.text.unwrap().contains("this is the body"));
        assert!(!m.has_html);
        assert!(m.refusal.is_none());
    }

    #[test]
    fn two_content_types_that_disagree_are_refused_rather_than_picked_between() {
        // THE test this module exists to make possible. The ambiguity rules can
        // only refuse a message whose duplicate headers they can SEE, so a
        // parser that keeps the first value and drops the rest would make the
        // whole of section 2 unreachable while every one of its unit tests
        // stayed green. This is that check, run against the real parser.
        let m = read(&msg(
            "Content-Type: text/plain\r\nContent-Type: text/html\r\n",
            "Which one am I?",
        ))
        .unwrap();
        let refusal = m.refusal.expect("a message claiming two content types must be refused");
        assert!(refusal.contains("Content-Type"), "the refusal must name the header: {refusal}");
        assert!(refusal.contains("text/plain") && refusal.contains("text/html"),
            "and quote both claims: {refusal}");
    }

    #[test]
    fn the_same_content_type_twice_is_not_a_disagreement() {
        // Repetition is not ambiguity, and refusing it would make the rule
        // useless on the mail that real senders produce.
        let m = read(&msg(
            "Content-Type: text/plain\r\nContent-Type: text/plain\r\n",
            "Said twice, meant once.",
        ))
        .unwrap();
        assert!(m.refusal.is_none(), "got {:?}", m.refusal);
    }

    #[test]
    fn a_header_that_phones_home_is_reported() {
        let m = read(&msg(
            "Content-Type: text/plain\r\nDisposition-Notification-To: watcher@example.com\r\n",
            "Read receipt requested.",
        ))
        .unwrap();
        assert!(
            m.channels.iter().any(|h| h.eq_ignore_ascii_case("Disposition-Notification-To")),
            "got {:?}", m.channels
        );
    }

    #[test]
    fn a_plain_message_does_not_claim_an_html_part_it_does_not_have() {
        // The convenience API says otherwise: `body_html(0)` renders the text
        // part and answers Some, so a surface asking it would tell the reader
        // every plain message carries HTML. Regression for the fix in `read`.
        let m = read(&msg("Content-Type: text/plain
", "No markup anywhere.")).unwrap();
        assert!(!m.has_html);
        assert!(m.divergence.is_none(), "and nothing to diverge from: {:?}", m.divergence);
    }

    #[test]
    fn a_real_html_part_is_seen_and_compared_against_the_text_one() {
        let raw = b"From: someone@example.com
Subject: Both
MIME-Version: 1.0
Content-Type: multipart/alternative; boundary=b

--b
Content-Type: text/plain

Pay the invoice at example.com
--b
Content-Type: text/html

<p>Pay the invoice at evil.example</p>
--b--
";
        let m = read(raw).unwrap();
        assert!(m.has_html);
        // The plan is explicit that a disagreement between the parts is
        // information about the message, not something to resolve quietly in
        // favour of one of them.
        assert!(m.divergence.is_some(), "the two parts name different hosts");
    }

    #[test]
    fn nothing_at_all_is_not_a_message() {
        assert!(read(b"").is_err());
    }
}
