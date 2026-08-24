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
    /// Who else it was addressed to, as written. Same rule as `from`: a display
    /// name is whatever the sender typed.
    ///
    /// Worth showing because a message to eight people and a message to you alone
    /// read very differently and looked identical here until 21 August. `Cc` is
    /// kept apart from `To` because the difference is the sender's statement about
    /// who the message is FOR, which is not the same list.
    pub to: Vec<String>,
    /// Who was copied, as written.
    pub cc: Vec<String>,
    /// The subject as written.
    pub subject: Option<String>,
    /// The date line as written.
    pub date: Option<String>,
    /// The `text/plain` body, when the message has one.
    pub text: Option<String>,
    /// Whether an HTML part exists. NOT its content: see the module note.
    pub has_html: bool,
    /// The words that appear ONLY in the text part, when both parts exist and
    /// they disagree. Empty when they agree or when there is only one part.
    ///
    /// DATA, not a sentence. This used to be a ready-made English string built
    /// in Rust and printed verbatim, which meant the one surface whose job is to
    /// let somebody judge a mismatch spoke English to a German reader. The words
    /// are the finding; the sentence around them belongs in the locale file.
    pub only_in_text: Vec<String>,
    /// The words that appear only in the formatted part. Same rule.
    pub only_in_html: Vec<String>,
    /// Why this message was refused, when the ambiguity rules refuse it.
    pub refusal: Option<String>,
    /// Headers that are themselves a way out of the machine.
    pub channels: Vec<String>,
    /// The scheme this message is sealed with, when it is sealed.
    ///
    /// A PGP or S/MIME message has no readable text part, so without this the
    /// window said "this message has no text" over two attachments with names
    /// like `encrypted.asc` - which describes an empty message rather than a
    /// sealed one. Nothing here decrypts anything; this only says which kind of
    /// seal is on it, so the surface can stop pretending it read the message.
    ///
    /// A SIGNED message is deliberately not in here: its text part is readable
    /// and is read, and calling it unreadable would hide a message somebody can
    /// have.
    pub sealed: Option<Sealed>,
    /// The invitation the message carries, when it carries one.
    ///
    /// NAMED, NOT READ. A `text/calendar` part is an invitation, a cancellation
    /// or a reply depending on its `method`, and turning one into an event means
    /// iTIP processing - which the plan's section 4 leaves as an open
    /// architectural call between this app and the calendar daemon. So this says
    /// the part is there and what the message claims it is FOR, and nothing in
    /// this crate parses the calendar payload. The calendar app already reads
    /// `.ics`; a second reader here would be a second set of differentials over
    /// the same bytes, which is the thing section 2 exists to prevent.
    pub invitation: Option<Invitation>,
    /// What the message CARRIES, named and measured, never opened.
    ///
    /// Same principle as `has_html`: say what is there without acting on it. A
    /// message with three files attached and one with none looked identical in
    /// the window until 21 August, which is a fact about somebody's mail that the
    /// surface simply did not mention. Nothing is extracted and nothing is
    /// written to disk by reading this.
    pub attachments: Vec<Attachment>,
}

/// How a message is sealed, as the message describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sealed {
    /// PGP/MIME: a `multipart/encrypted` whose protocol names PGP.
    Pgp,
    /// S/MIME: an `application/pkcs7-mime` enveloped part.
    Smime,
    /// A `multipart/encrypted` whose protocol is something else, or absent. Named
    /// rather than guessed at: the message says it is sealed and does not say
    /// with what this reader recognises.
    Unknown,
}

/// A calendar part, as the message describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invitation {
    /// The `method` parameter as written, lowercased: `request`, `cancel`,
    /// `reply`, `publish` and so on. `None` when the part named none, which is
    /// legal and means the surface must not claim it is an invitation to
    /// something - only that the message carries a calendar part.
    pub method: Option<String>,
    /// How many bytes the part decodes to. The same measured-not-opened rule as
    /// an attachment.
    pub bytes: usize,
    /// The filename the part gave, when it gave one.
    ///
    /// Present so a surface can tell that the invitation and one of the rows in
    /// `attachments` are THE SAME PART. The common shape from Outlook is a
    /// `text/calendar` part with `Content-Disposition: attachment;
    /// filename=invite.ics`, and it lands in both lists - measured, not guessed
    /// at. Both statements are true, and a window that wants to avoid saying
    /// "carries an invitation" and "carries one file" about one thing needs a
    /// way to join them up, which is this rather than matching on media type and
    /// hoping.
    pub filename: Option<String>,
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

/// How this message is sealed, if it is. Extracted so the attachment list and a
/// reader of one attachment's bytes cannot disagree about which parts are the
/// envelope: they ask the same function.
fn sealed_of(parsed: &mail_parser::Message<'_>) -> Option<Sealed> {
    parsed.parts.iter().find_map(|part| {
        let ct = part.content_type()?;
        let ctype = ct.ctype();
        let subtype = ct.subtype().unwrap_or_default();
        if ctype.eq_ignore_ascii_case("multipart") && subtype.eq_ignore_ascii_case("encrypted") {
            let protocol = ct.attribute("protocol").unwrap_or_default().to_ascii_lowercase();
            return Some(if protocol.contains("pgp") { Sealed::Pgp } else { Sealed::Unknown });
        }
        if ctype.eq_ignore_ascii_case("application")
            && subtype.eq_ignore_ascii_case("pkcs7-mime")
        {
            // `signed-data` is a signature wrapper, not a seal a reader must give
            // up on - but its content is still not text this app can show, so it
            // is named too rather than left as an empty message.
            return Some(Sealed::Smime);
        }
        None
    })
}

/// The `text/calendar` part's index, if the message carries one. Same reason as
/// [`sealed_of`]: one answer, two readers.
fn calendar_part_of(parsed: &mail_parser::Message<'_>) -> Option<usize> {
    parsed.parts.iter().position(|part| {
        part.content_type().is_some_and(|ct| {
            ct.ctype().eq_ignore_ascii_case("text")
                && ct.subtype().is_some_and(|s| s.eq_ignore_ascii_case("calendar"))
        })
    })
}

/// The parts `read` lists as attachments, in the order it lists them, as indices
/// into `parsed.parts`.
///
/// The INDEX IS A CONTRACT: the surface presses "save" on attachment 2, and this
/// is what says which part that is. Written once and used by both, because two
/// copies of a filter chain that agree today are two copies that will not.
fn attachment_part_indices(parsed: &mail_parser::Message<'_>) -> Vec<usize> {
    let sealed = sealed_of(parsed);
    let calendar_part = calendar_part_of(parsed);
    parsed
        .attachments()
        .filter(|_| sealed.is_none())
        .filter(|part| calendar_part.is_none_or(|i| !std::ptr::eq(*part, &parsed.parts[i])))
        .filter_map(|part| parsed.parts.iter().position(|p| std::ptr::eq(p, part)))
        .collect()
}

/// One attachment's decoded bytes, by the index [`read`] listed it under.
///
/// `None` when the message does not parse or carries no such attachment. The
/// caller writes the file; this only reads the part, and it does NOT touch the
/// sender's filename - see [`Attachment::name`] for why that string is a
/// suggestion and not a destination.
pub fn attachment_bytes(raw: &[u8], index: usize) -> Option<Vec<u8>> {
    let parsed = mail_parser::MessageParser::default().parse(raw)?;
    let at = *attachment_part_indices(&parsed).get(index)?;
    Some(parsed.parts[at].contents().to_vec())
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
    let (only_in_text, only_in_html) = match (&text, &html) {
        (Some(t), Some(h)) => {
            let d = alternative::compare(t, h);
            (d.sample_text(), d.sample_html())
        }
        _ => (Vec::new(), Vec::new()),
    };

    // Named and measured, never opened: `contents()` is the decoded bytes and the
    // only thing taken from them is the length.
    // The calendar part, if there is one. Every part is examined rather than the
    // first: a message can carry a plain body, an HTML body and an invitation,
    // and an invitation is often not flagged as an attachment, so the attachment
    // list above does not necessarily mention it.
    // SEALED, not empty. Checked over every part rather than the top level alone,
    // because an S/MIME message carries its envelope as the body part and a
    // PGP/MIME one declares it on the message itself.
    let sealed = sealed_of(&parsed);

    let calendar_part = calendar_part_of(&parsed);

    // The invitation is NOT listed again here. `attachments()` includes a
    // `text/calendar` part whether or not the sender marked it as one, so the
    // window said "carries an invitation" and "carries one file, not opened: a
    // file the sender did not name, text/calendar" about a single part - two
    // arrivals where there was one, and the second sentence knows less than the
    // first. Reported once, as what it is; its size is on the invitation.
    // A SEALED message carries no files, it carries a seal. Its outer parts are
    // the envelope - PGP/MIME's version part and the ciphertext, or S/MIME's
    // single `pkcs7-mime` - and listing them as "2 files, not opened" tells a
    // person they received two files when they received one message they cannot
    // read. Anything the sender actually attached is INSIDE the ciphertext and
    // nothing out here can see it, so there is nothing being hidden by this.
    let attachments: Vec<Attachment> = parsed
        .attachments()
        .filter(|_| sealed.is_none())
        .filter(|part| {
            calendar_part.is_none_or(|i| !std::ptr::eq(*part, &parsed.parts[i]))
        })
        .map(|part| Attachment {
            name: part.attachment_name().map(str::to_string),
            media_type: part.content_type().map(|c| match c.subtype() {
                Some(sub) => format!("{}/{}", c.ctype(), sub),
                None => c.ctype().to_string(),
            }),
            bytes: part.contents().len(),
        })
        .collect();

    let invitation = calendar_part.map(|i| {
        let part = &parsed.parts[i];
        let ct = part.content_type();
        Invitation {
            method: ct
                .and_then(|c| c.attribute("method"))
                .map(|m| m.to_ascii_lowercase()),
            bytes: part.contents().len(),
            filename: part.attachment_name().map(str::to_string),
        }
    });

    // Every address in the header, not the first: a reader that keeps one has
    // decided the message was addressed to one person.
    let addresses = |list: Option<&mail_parser::Address>| -> Vec<String> {
        list.map(|a| {
            a.iter()
                .filter_map(|addr| addr.address().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
    };

    Ok(Message {
        to: addresses(parsed.to()),
        cc: addresses(parsed.cc()),
        from: parsed.from().and_then(|a| a.first()).and_then(|a| a.address().map(str::to_string)),
        subject: parsed.subject().map(str::to_string),
        date: parsed.date().map(|d| d.to_rfc3339()),
        text,
        has_html: html.is_some(),
        only_in_text,
        only_in_html,
        sealed,
        invitation,
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
    fn every_recipient_is_kept_and_copied_ones_stay_apart() {
        // Eight people and one person read very differently, and keeping only the
        // first address decides for the reader that the message was to them alone.
        let raw = b"From: a@example.org\r\n\
To: one@example.org, two@example.org\r\n\
Cc: three@example.org\r\n\
Subject: several\r\n\
\r\n\
body\r\n";
        let m = read(raw).unwrap();
        assert_eq!(m.to, ["one@example.org", "two@example.org"]);
        assert_eq!(m.cc, ["three@example.org"]);
    }

    #[test]
    fn a_message_with_no_recipient_header_carries_none_rather_than_an_empty_name() {
        let m = read(b"From: a@example.org\r\nSubject: x\r\n\r\nbody\r\n").unwrap();
        assert!(m.to.is_empty());
        assert!(m.cc.is_empty());
    }

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
        assert!(
            m.only_in_text.is_empty() && m.only_in_html.is_empty(),
            "and nothing to diverge from: {:?} {:?}",
            m.only_in_text,
            m.only_in_html
        );
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
        assert!(
            !m.only_in_text.is_empty() || !m.only_in_html.is_empty(),
            "the two parts name different hosts"
        );
    }

    #[test]
    fn an_invitation_is_named_but_not_read() {
        let raw = b"From: ada@example.org\r\n\
Subject: Lunch\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/alternative; boundary=b\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain\r\n\
\r\n\
Lunch on Friday?\r\n\
--b\r\n\
Content-Type: text/calendar; method=REQUEST; charset=utf-8\r\n\
\r\n\
BEGIN:VCALENDAR\r\n\
END:VCALENDAR\r\n\
--b--\r\n";
        let m = read(raw).unwrap();
        let inv = m.invitation.expect("the message carries a calendar part");
        // Lowercased, because a sender writes REQUEST or Request and a surface
        // that switches on the casing shows nothing for one of them.
        assert_eq!(inv.method.as_deref(), Some("request"));
        assert!(inv.bytes > 0);
    }

    #[test]
    fn a_calendar_part_with_no_method_still_shows_as_one() {
        let raw = b"From: ada@example.org\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/calendar\r\n\
\r\n\
BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n";
        let inv = read(raw).unwrap().invitation.expect("still a calendar part");
        // No method means the surface may say the message carries a calendar
        // part and may NOT say what it is asking for.
        assert_eq!(inv.method, None);
    }

    #[test]
    fn an_invitation_sent_as_an_attachment_is_reported_once() {
        // The Outlook shape, and also the ordinary one: `attachments()` returns a
        // `text/calendar` part whether or not the sender marked it as an
        // attachment. Listing it in both places made the window say "carries an
        // invitation" and "carries one file, not opened: a file the sender did
        // not name, text/calendar" about a single part - two arrivals where there
        // was one, and the second sentence knows less than the first.
        let raw = b"From: ada@example.org\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\n\
Lunch?\r\n--b\r\nContent-Type: text/calendar; method=REQUEST; name=invite.ics\r\n\
Content-Disposition: attachment; filename=invite.ics\r\n\r\n\
BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--b--\r\n";
        let m = read(raw).unwrap();
        let inv = m.invitation.expect("an invitation");
        assert_eq!(inv.filename.as_deref(), Some("invite.ics"));
        // Named once, as what it is. Its size is on the invitation.
        assert!(m.attachments.is_empty(), "{:?}", m.attachments);
        assert!(inv.bytes > 0);
    }

    #[test]
    fn the_bytes_come_from_the_part_the_list_named() {
        // The index is the contract between the list and the save button, and a
        // calendar part sitting BETWEEN two attachments is what breaks a second
        // enumeration that filters differently: `attachments()` would hand back
        // three parts where `read` listed two, and saving the second would write
        // the invitation.
        let raw = b"From: ada@example.org\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\n\
Lunch?\r\n--b\r\nContent-Type: application/pdf\r\n\
Content-Disposition: attachment; filename=first.pdf\r\n\r\nFIRST\r\n\
--b\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\n\
BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--b\r\nContent-Type: application/pdf\r\n\
Content-Disposition: attachment; filename=second.pdf\r\n\r\nSECOND\r\n--b--\r\n";
        let m = read(raw).unwrap();
        assert_eq!(m.attachments.len(), 2);
        assert_eq!(m.attachments[0].name.as_deref(), Some("first.pdf"));
        assert_eq!(m.attachments[1].name.as_deref(), Some("second.pdf"));

        let first = attachment_bytes(raw, 0).expect("attachment 0");
        let second = attachment_bytes(raw, 1).expect("attachment 1");
        assert!(String::from_utf8_lossy(&first).contains("FIRST"), "{first:?}");
        assert!(
            String::from_utf8_lossy(&second).contains("SECOND"),
            "the second listed attachment must be the second saved: {second:?}"
        );
        assert!(attachment_bytes(raw, 2).is_none(), "there is no third");
    }

    #[test]
    fn a_sealed_message_hands_out_no_bytes() {
        // It lists no attachments, so there is no index to ask about - and the
        // parts out here are the envelope rather than anything the sender sent.
        let raw = b"From: ada@example.org\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=b\r\n\r\n\
--b\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\
--b\r\nContent-Type: application/octet-stream\r\n\r\nCIPHER\r\n--b--\r\n";
        let m = read(raw).unwrap();
        assert!(m.attachments.is_empty());
        assert!(attachment_bytes(raw, 0).is_none());
    }

    #[test]
    fn a_real_attachment_beside_an_invitation_is_still_listed() {
        // The exclusion is of THAT part, not of attachments in general.
        let raw = b"From: ada@example.org\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\n\
Lunch?\r\n--b\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\n\
BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n--b\r\nContent-Type: application/pdf\r\n\
Content-Disposition: attachment; filename=menu.pdf\r\n\r\n%PDF-1.4\r\n--b--\r\n";
        let m = read(raw).unwrap();
        assert!(m.invitation.is_some());
        assert_eq!(m.attachments.len(), 1);
        assert_eq!(m.attachments[0].name.as_deref(), Some("menu.pdf"));
    }

    #[test]
    fn a_pgp_message_is_named_as_sealed_rather_than_empty() {
        let raw = b"From: ada@example.org\r\nSubject: Secret\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=b\r\n\r\n\
--b\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\
--b\r\nContent-Type: application/octet-stream; name=encrypted.asc\r\n\r\n\
-----BEGIN PGP MESSAGE-----\r\n-----END PGP MESSAGE-----\r\n--b--\r\n";
        let m = read(raw).unwrap();
        assert_eq!(m.sealed, Some(Sealed::Pgp));
        // The point of the field: the text really is absent, and without the
        // seal the window would report an empty message.
        assert!(m.text.is_none());
    }

    #[test]
    fn an_smime_envelope_is_named_too() {
        let raw = b"From: a@b\r\nMIME-Version: 1.0\r\n\
Content-Type: application/pkcs7-mime; smime-type=enveloped-data; name=smime.p7m\r\n\
Content-Transfer-Encoding: base64\r\n\r\nMIAGCSqGSIb3DQEHA6CA\r\n";
        assert_eq!(read(raw).unwrap().sealed, Some(Sealed::Smime));
    }

    #[test]
    fn a_sealed_message_does_not_report_its_envelope_as_files() {
        let raw = b"From: ada@example.org\r\nSubject: Secret\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=b\r\n\r\n\
--b\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\
--b\r\nContent-Type: application/octet-stream; name=encrypted.asc\r\n\r\n\
-----BEGIN PGP MESSAGE-----\r\n-----END PGP MESSAGE-----\r\n--b--\r\n";
        let m = read(raw).unwrap();
        assert_eq!(m.sealed, Some(Sealed::Pgp));
        // Both outer parts are the envelope. The window said "This message
        // carries 2 files, not opened" over them, which is a sentence about
        // enclosures nobody sent.
        assert!(m.attachments.is_empty(), "{:?}", m.attachments);
    }

    #[test]
    fn a_signed_message_is_not_called_sealed() {
        // Its text is readable and IS read. Calling this unreadable would hide a
        // message somebody can have, which is the opposite mistake.
        let raw = b"From: a@b\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=b\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nHello, this is readable.\r\n\
--b\r\nContent-Type: application/pgp-signature\r\n\r\n\
-----BEGIN PGP SIGNATURE-----\r\n--b--\r\n";
        let m = read(raw).unwrap();
        assert_eq!(m.sealed, None);
        assert_eq!(m.text.as_deref(), Some("Hello, this is readable."));
    }

    #[test]
    fn a_message_without_one_says_so() {
        let raw = b"From: ada@example.org\r\nSubject: Hello\r\n\r\nNo calendar here.\r\n";
        assert_eq!(read(raw).unwrap().invitation, None);
    }

    #[test]
    fn nothing_at_all_is_not_a_message() {
        assert!(read(b"").is_err());
    }
}
