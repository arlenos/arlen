//! The receive-side confused-deputy defense (profile-system-plan.md, Decided 4).
//!
//! The hazard a directional broker must close is the RECEIVING parser: a
//! malicious source profile could hand the destination a poisoned document that
//! the destination's own parser executes. The defense reuses the content-origin
//! + parse-sandbox frame already built for the AI layer:
//!
//! - Every payload crossing into the destination profile is stamped
//!   [`Origin::ExternalContent`] - the destination must treat cross-profile
//!   bytes as the highest-risk origin, never as its own trusted data. A
//!   [`Delivery`] on the cross-profile path has no constructor that omits the
//!   stamp, so an unstamped cross-profile delivery is unrepresentable.
//! - Any `File` document payload must route through the document-parse sandbox
//!   (S18-B `ai_sandbox::parse_document`) on the RECEIVING side before any
//!   destination consumer reads text. This module owns the pure routing DECISION
//!   ([`requires_parse_sandbox`]); the live `parse_document` call lands on the
//!   deferred deliver path (it needs the sandbox bin path and the live bytes,
//!   both on the broker path). The receive guard is fail-closed: if
//!   `parse_document` errs, no text is delivered (the ai-sandbox contract - any
//!   error means no trustworthy text, so pass nothing).
//!
//! `Origin` here mirrors `ai_core::tagging::Origin` faithfully. It is duplicated
//! rather than depended on because this daemon is a standalone workspace and the
//! tagging crate lives in the `ai/` workspace; the live deliver path uses the
//! canonical `ai_core` type and the real `ai_sandbox::parse_document`. The two
//! must stay in step: a cross-profile delivery is `ExternalContent`, full stop.

use crate::request::{PayloadRef, TransferType};

/// The provenance of delivered bytes. Mirrors `ai_core::tagging::Origin`; a
/// cross-profile delivery is always [`Origin::ExternalContent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The highest-risk origin: content from outside the destination profile's
    /// own trust domain. Cross-profile transfers are always this.
    ExternalContent,
}

impl Origin {
    /// The tag label, matching `ai_core::tagging::Origin::label`.
    pub fn label(self) -> &'static str {
        match self {
            Origin::ExternalContent => "EXTERNAL-CONTENT",
        }
    }
}

/// A payload shaped for delivery into the destination profile. Built only via
/// [`Delivery::cross_profile`], which stamps [`Origin::ExternalContent`]
/// unconditionally, so a cross-profile delivery can never reach a destination
/// consumer untagged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delivery {
    origin: Origin,
    ty: TransferType,
    is_document: bool,
}

impl Delivery {
    /// Shape a cross-profile delivery. The origin is stamped
    /// [`Origin::ExternalContent`] with no opt-out. `is_document` marks a `File`
    /// payload whose bytes are a parseable document (so it must route through
    /// the sandbox); a clipboard text selection is not a document.
    pub fn cross_profile(payload: &PayloadRef, is_document: bool) -> Self {
        let ty = match payload {
            PayloadRef::Clipboard { .. } => TransferType::Clipboard,
            PayloadRef::File { .. } => TransferType::File,
        };
        Self {
            origin: Origin::ExternalContent,
            ty,
            // Only a File payload can carry a parseable document.
            is_document: matches!(payload, PayloadRef::File { .. }) && is_document,
        }
    }

    /// The (always [`Origin::ExternalContent`]) origin stamp.
    pub fn origin(&self) -> Origin {
        self.origin
    }

    /// The flow type of the delivered payload.
    pub fn ty(&self) -> TransferType {
        self.ty
    }
}

/// Whether this delivery's bytes MUST go through the S18-B document-parse
/// sandbox before any destination consumer reads them.
///
/// True for a `File` document payload: the destination's own parser must never
/// see the raw cross-profile bytes; only the inert stripped text the sandbox
/// returns may be delivered. A `Clipboard` text selection is stamped
/// `ExternalContent` (the destination treats it as untrusted text) but does not
/// need the document sandbox - it is not a parsed document.
pub fn requires_parse_sandbox(delivery: &Delivery) -> bool {
    delivery.ty == TransferType::File && delivery.is_document
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cross_profile_delivery_is_always_external_content() {
        let clip = Delivery::cross_profile(&PayloadRef::Clipboard { handle: "h".into() }, false);
        assert_eq!(clip.origin(), Origin::ExternalContent);
        let file = Delivery::cross_profile(
            &PayloadRef::File {
                source_path: "/p".into(),
            },
            true,
        );
        assert_eq!(file.origin(), Origin::ExternalContent);
    }

    #[test]
    fn a_file_document_requires_the_sandbox() {
        let doc = Delivery::cross_profile(
            &PayloadRef::File {
                source_path: "/work/report.pdf".into(),
            },
            true,
        );
        assert!(
            requires_parse_sandbox(&doc),
            "a document File must route through S18-B",
        );
    }

    #[test]
    fn clipboard_text_is_external_but_needs_no_document_sandbox() {
        let clip = Delivery::cross_profile(&PayloadRef::Clipboard { handle: "h".into() }, false);
        assert_eq!(clip.origin(), Origin::ExternalContent);
        assert!(
            !requires_parse_sandbox(&clip),
            "clipboard text is untrusted but is not a parsed document",
        );
    }

    #[test]
    fn a_non_document_file_skips_the_sandbox_but_stays_external() {
        // A File payload not marked as a document (e.g. an opaque blob the
        // destination will not parse) is still ExternalContent.
        let blob = Delivery::cross_profile(
            &PayloadRef::File {
                source_path: "/work/data.bin".into(),
            },
            false,
        );
        assert_eq!(blob.origin(), Origin::ExternalContent);
        assert!(!requires_parse_sandbox(&blob));
    }

    #[test]
    fn the_external_content_label_matches_the_canonical_tag() {
        // Must stay in step with ai_core::tagging::Origin::ExternalContent.
        assert_eq!(Origin::ExternalContent.label(), "EXTERNAL-CONTENT");
    }
}
