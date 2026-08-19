// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Every way a message can make this machine phone home, and where we stand on
//! each.
//!
//! `mail-app.md` section 3 sets a condition on a sentence every mail client
//! prints: **any "block remote content" claim must be measured against this
//! whole list, or it is a half-truth.** The EFAIL work is the source, and its
//! finding is that mail exfiltration is a client-architecture problem rather
//! than a crypto one - direct exfiltration needs no cryptographic weakness at
//! all, has worked since 2003, and does not depend on HTML.
//!
//! So the list lives here as data rather than as prose in a design document,
//! with our position on each channel beside it. Two things follow. A surface can
//! render the honest sentence instead of the comforting one, naming what is
//! actually stopped. And when somebody later writes "remote content is blocked"
//! in a settings page, the claim has something to be checked against.
//!
//! **Most of these are not blocked today, and that is what this says.** There is
//! no renderer yet; a stance of [`Stance::Open`] means exactly that nothing
//! stands in the way, and it is recorded so it cannot be mistaken for a decision
//! that was made and forgotten.

/// A way a message can reach the network while being read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    /// `<img src="https://...">`: the one everybody means by "remote content".
    ImageSource,
    /// CSS `@import`, which fetches on its own and is not an image.
    CssImport,
    /// `<object data="ftp://...">` and its relatives.
    ObjectData,
    /// Script in the message.
    Script,
    /// `Disposition-Notification-To`: a read receipt the client offers to send.
    ReadReceiptHeader,
    /// `Remote-Attachment-URL`: an attachment the client fetches on open.
    RemoteAttachmentHeader,
    /// `X-Image-URL`: a sender image fetched to decorate the message.
    SenderImageHeader,
    /// Previewing an attachment that can itself fetch - PDF, SVG, vCard.
    AttachmentPreview,
    /// Checking a signature: CRL lookups and intermediate-certificate fetches.
    ///
    /// The one nobody expects. Ten clients in the study made CRL requests and
    /// seven fetched intermediates, which means the mail client phoned out while
    /// merely verifying that a message was signed.
    CertificateValidation,
}

/// Where this client stands on a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stance {
    /// Nothing stops it. The honest default until something does.
    Open,
    /// Refused, with the mechanism that refuses it named.
    Blocked(&'static str),
    /// Cannot arise here, with the reason.
    NotApplicable(&'static str),
}

impl Channel {
    /// Every channel the plan names. The list is the point.
    #[must_use]
    pub fn all() -> Vec<Self> {
        vec![
            Self::ImageSource,
            Self::CssImport,
            Self::ObjectData,
            Self::Script,
            Self::ReadReceiptHeader,
            Self::RemoteAttachmentHeader,
            Self::SenderImageHeader,
            Self::AttachmentPreview,
            Self::CertificateValidation,
        ]
    }

    /// What this channel is, in a sentence a reader could be shown.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::ImageSource => "images loaded from the sender's server",
            Self::CssImport => "styles fetched by the message's own stylesheet",
            Self::ObjectData => "embedded objects that fetch their own contents",
            Self::Script => "script inside the message",
            Self::ReadReceiptHeader => "a read receipt the message asks to be sent",
            Self::RemoteAttachmentHeader => "an attachment stored elsewhere and fetched on open",
            Self::SenderImageHeader => "a sender picture fetched to decorate the message",
            Self::AttachmentPreview => "a previewed attachment that fetches on its own",
            Self::CertificateValidation => "checking the signature, which asks a certificate authority",
        }
    }

    /// Where we stand, today.
    ///
    /// Every answer is `Open` because there is no renderer and no fetch path
    /// yet. Writing them out anyway is the point: an empty list would read as
    /// "nothing to worry about", and a channel nobody has thought about looks
    /// exactly like a channel that was handled.
    #[must_use]
    pub fn stance(self) -> Stance {
        Stance::Open
    }
}

/// A header-borne channel a specific message carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Which channel.
    pub channel: Channel,
    /// The header that carried it, as written.
    pub header: String,
    /// What it pointed at.
    pub value: String,
}

/// The header-borne channels in one message.
///
/// These three are checkable now, before any renderer exists, because they are
/// literally header names - which is also why they are the ones clients forget:
/// a message can reach the network through a client that renders no HTML at all.
#[must_use]
pub fn header_channels(headers: &[(String, String)]) -> Vec<Found> {
    headers
        .iter()
        .filter_map(|(name, value)| {
            let channel = match name.trim().to_lowercase().as_str() {
                "disposition-notification-to" => Channel::ReadReceiptHeader,
                "remote-attachment-url" => Channel::RemoteAttachmentHeader,
                "x-image-url" => Channel::SenderImageHeader,
                _ => return None,
            };
            Some(Found { channel, header: name.clone(), value: value.clone() })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_channel_the_plan_names_is_in_the_list() {
        // The list shrinking is the failure mode: a channel dropped from here is
        // a channel a future "we block remote content" claim silently excludes.
        assert_eq!(Channel::all().len(), 9);
        for c in Channel::all() {
            assert!(!c.describe().is_empty(), "{c:?} has no description");
        }
    }

    #[test]
    fn nothing_claims_to_be_blocked_while_nothing_blocks_it() {
        // The half-truth guard, as a test. When a renderer lands and starts
        // refusing these, this test is what has to be edited - deliberately, one
        // channel at a time, naming the mechanism.
        for c in Channel::all() {
            assert_eq!(c.stance(), Stance::Open, "{c:?} claims a stance nothing implements");
        }
    }

    #[test]
    fn a_read_receipt_request_is_found_in_the_headers() {
        let found = header_channels(&[
            ("Subject".into(), "Invoice".into()),
            ("Disposition-Notification-To".into(), "tracker@example.com".into()),
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].channel, Channel::ReadReceiptHeader);
        assert_eq!(found[0].value, "tracker@example.com");
    }

    #[test]
    fn the_header_names_are_matched_however_they_are_cased() {
        // Header names are case-insensitive, and a sender who wants the fetch
        // will write them however gets past a naive match.
        let found = header_channels(&[("x-IMAGE-url".into(), "https://t.example/1.png".into())]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].channel, Channel::SenderImageHeader);
    }

    #[test]
    fn a_message_can_carry_several_at_once() {
        let found = header_channels(&[
            ("Disposition-Notification-To".into(), "a@x".into()),
            ("Remote-Attachment-URL".into(), "https://x/y".into()),
            ("X-Image-URL".into(), "https://x/z".into()),
        ]);
        assert_eq!(found.len(), 3);
    }

    #[test]
    fn an_ordinary_message_carries_none() {
        let found = header_channels(&[
            ("From".into(), "a@x".into()),
            ("Subject".into(), "Lunch".into()),
        ]);
        assert!(found.is_empty());
    }
}
