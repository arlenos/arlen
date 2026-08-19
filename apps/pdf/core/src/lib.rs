// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! What a PDF says about itself, before anything is drawn.
//!
//! `quickview-plan.md` puts the PDF reader in its own app and names its feature
//! set: page navigation, an outline, in-document search, text selection. Three
//! of those four are questions about the DOCUMENT rather than about pixels, and
//! this crate answers them: how many pages there are, what the author's own
//! table of contents says, and what text each page carries.
//!
//! **Rendering is deliberately absent.** It needs PDFium, which is a system
//! library, which is a new line in `ci-system-packages.txt` - the file the apt
//! cache key hashes - so adding it makes every CI job cold. That is a cost worth
//! paying once, on purpose, for the rasteriser; it is not worth paying as a side
//! effect of starting the reader. Everything here is structure and text.
//!
//! **Fail-closed, in a specific sense.** A PDF is a file somebody was sent, so
//! every entry point takes bytes and returns a `Result`; a malformed document is
//! an answer, not a panic. Where the format allows something to be absent - no
//! outline, a page with no text - that is reported as absent rather than as an
//! error, because "this document has no table of contents" is a true statement a
//! reader has to be able to make.

use std::collections::HashSet;

use lopdf::{Document as LoDocument, Object, ObjectId};

/// What went wrong with a document somebody handed us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PdfError {
    /// The bytes are not a PDF, or are damaged past reading.
    Unreadable(String),
    /// It parsed, but carries no pages at all.
    NoPages,
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreadable(why) => write!(f, "this file could not be read as a PDF: {why}"),
            Self::NoPages => write!(f, "this PDF contains no pages"),
        }
    }
}

impl std::error::Error for PdfError {}

/// One line of the author's own table of contents.
///
/// Flat, with a `depth`, rather than a tree of children. A reader draws an
/// indented list and jumps to a page; a tree would have to be flattened at every
/// use, and the nesting is the only thing the tree carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineEntry {
    /// The title as the document wrote it.
    pub title: String,
    /// How deep it sits, zero for a top-level heading.
    pub depth: usize,
    /// The page it points at, one-based, when the document says.
    ///
    /// `None` is a real answer: an outline entry may target a named destination
    /// this crate has not resolved, and an entry that cannot say where it goes
    /// is still worth showing. A reader disables the jump rather than hiding the
    /// heading.
    pub page: Option<usize>,
}

/// A PDF, read for what it says rather than for how it looks.
#[derive(Debug)]
pub struct Document {
    inner: LoDocument,
    /// Page object ids in reading order, so a one-based page number can be
    /// resolved without asking lopdf to re-walk the tree each time.
    pages: Vec<ObjectId>,
}

impl Document {
    /// Read a document from bytes.
    ///
    /// # Errors
    /// [`PdfError::Unreadable`] when the bytes do not parse, [`PdfError::NoPages`]
    /// when they parse into a document with no pages - which is a file that
    /// cannot be shown, and saying so beats opening an empty window.
    pub fn open(bytes: &[u8]) -> Result<Self, PdfError> {
        let inner =
            LoDocument::load_mem(bytes).map_err(|e| PdfError::Unreadable(e.to_string()))?;
        let mut pages: Vec<(u32, ObjectId)> = inner.get_pages().into_iter().collect();
        // `get_pages` hands back a map keyed by page number; sorted here so the
        // vector's index IS the reading order rather than a hash order.
        pages.sort_by_key(|(number, _)| *number);
        let pages: Vec<ObjectId> = pages.into_iter().map(|(_, id)| id).collect();
        if pages.is_empty() {
            return Err(PdfError::NoPages);
        }
        Ok(Self { inner, pages })
    }

    /// How many pages there are. Always at least one, by construction.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// The author's table of contents, top to bottom, or empty when there is none.
    ///
    /// Empty is a real answer and the caller must be able to tell it apart from a
    /// failure, which is why this cannot fail: a document with no `/Outlines` is
    /// simply a document nobody wrote a contents page for.
    #[must_use]
    pub fn outline(&self) -> Vec<OutlineEntry> {
        let mut out = Vec::new();
        let Some(first) = self.outline_root() else {
            return out;
        };
        // A malformed document can point an outline entry back at one already
        // walked. Following that is an infinite list, so every id is visited at
        // most once and a loop simply ends.
        let mut seen = HashSet::new();
        self.walk_outline(first, 0, &mut seen, &mut out);
        out
    }

    /// The first top-level outline entry, when the catalogue names one.
    fn outline_root(&self) -> Option<ObjectId> {
        let catalog = self.inner.catalog().ok()?;
        let outlines = catalog.get(b"Outlines").ok()?;
        let dict = match outlines {
            Object::Reference(id) => self.inner.get_dictionary(*id).ok()?,
            Object::Dictionary(d) => d,
            _ => return None,
        };
        match dict.get(b"First").ok()? {
            Object::Reference(id) => Some(*id),
            _ => None,
        }
    }

    /// Walk one sibling chain, descending into children as it goes.
    fn walk_outline(
        &self,
        start: ObjectId,
        depth: usize,
        seen: &mut HashSet<ObjectId>,
        out: &mut Vec<OutlineEntry>,
    ) {
        let mut current = Some(start);
        while let Some(id) = current {
            if !seen.insert(id) {
                return;
            }
            let Ok(dict) = self.inner.get_dictionary(id) else {
                return;
            };
            if let Some(title) = dict.get(b"Title").ok().and_then(|t| self.text_of(t)) {
                out.push(OutlineEntry { title, depth, page: self.destination_page(dict) });
            }
            if let Some(Object::Reference(child)) = dict.get(b"First").ok() {
                self.walk_outline(*child, depth + 1, seen, out);
            }
            current = match dict.get(b"Next").ok() {
                Some(Object::Reference(next)) => Some(*next),
                _ => None,
            };
        }
    }

    /// The one-based page an outline entry points at, when it points directly.
    ///
    /// Only the direct form is resolved: `/Dest` as an array whose first element
    /// is a page reference. A named destination is an indirection through the
    /// document's name tree and resolves to `None` here - the entry still shows,
    /// with its jump disabled, which is the honest rendering of "the document
    /// knows where this goes and we do not".
    fn destination_page(&self, dict: &lopdf::Dictionary) -> Option<usize> {
        let dest = match dict.get(b"Dest").ok()? {
            Object::Array(a) => a.clone(),
            // An action-based entry keeps its target under `/A`.
            Object::Reference(id) => match self.inner.get_object(*id).ok()? {
                Object::Array(a) => a.clone(),
                _ => return None,
            },
            _ => {
                let action = dict.get(b"A").ok()?;
                let action = match action {
                    Object::Reference(id) => self.inner.get_dictionary(*id).ok()?,
                    Object::Dictionary(d) => d,
                    _ => return None,
                };
                match action.get(b"D").ok()? {
                    Object::Array(a) => a.clone(),
                    _ => return None,
                }
            }
        };
        let target = match dest.first()? {
            Object::Reference(id) => *id,
            _ => return None,
        };
        self.pages.iter().position(|p| *p == target).map(|i| i + 1)
    }

    /// A PDF string as text, whichever of the two encodings it used.
    ///
    /// UTF-16BE when it carries the byte-order mark the format prescribes, and
    /// PDFDoc-ish otherwise. Lossy on purpose: a title with one unrepresentable
    /// byte should still appear in the contents rather than vanish from it.
    fn text_of(&self, object: &Object) -> Option<String> {
        let bytes = match object {
            Object::String(b, _) => b.clone(),
            Object::Reference(id) => match self.inner.get_object(*id).ok()? {
                Object::String(b, _) => b.clone(),
                _ => return None,
            },
            _ => return None,
        };
        if bytes.starts_with(&[0xFE, 0xFF]) {
            let units: Vec<u16> = bytes[2..]
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        } else {
            Some(String::from_utf8_lossy(&bytes).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::content::{Content, Operation};
    use lopdf::{dictionary, Stream};

    /// A real PDF, built here rather than committed as a binary fixture: a test
    /// that reads a file nobody can diff is a test nobody can correct.
    fn pdf_with(pages_text: &[&str], outline: bool) -> Vec<u8> {
        let mut doc = LoDocument::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let resources =
            doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });

        let mut page_ids = Vec::new();
        for text in pages_text {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 12.into()]),
                    Operation::new("Td", vec![10.into(), 700.into()]),
                    Operation::new("Tj", vec![Object::string_literal(*text)]),
                    Operation::new("ET", vec![]),
                ],
            };
            let stream = doc.add_object(Stream::new(
                dictionary! {},
                content.encode().expect("content encodes"),
            ));
            let page = doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id,
                "Contents" => stream, "Resources" => resources,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            });
            page_ids.push(page);
        }

        let kids: Vec<Object> = page_ids.iter().map(|id| (*id).into()).collect();
        let count = i64::try_from(page_ids.len()).expect("page count fits");
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => kids, "Count" => count,
            }),
        );

        let mut catalog = dictionary! { "Type" => "Catalog", "Pages" => pages_id };
        if outline {
            // Two top-level headings, the first with a child, so depth and the
            // sibling chain are both exercised.
            let outlines_id = doc.new_object_id();
            let child = doc.add_object(dictionary! {
                "Title" => Object::string_literal("Background"),
                "Parent" => outlines_id,
                "Dest" => vec![page_ids[0].into(), "XYZ".into()],
            });
            let second = doc.new_object_id();
            let first = doc.add_object(dictionary! {
                "Title" => Object::string_literal("Introduction"),
                "Parent" => outlines_id,
                "First" => child, "Last" => child, "Next" => second,
                "Dest" => vec![page_ids[0].into(), "XYZ".into()],
            });
            doc.objects.insert(
                second,
                Object::Dictionary(dictionary! {
                    "Title" => Object::string_literal("Method"),
                    "Parent" => outlines_id,
                    "Dest" => vec![page_ids[page_ids.len() - 1].into(), "XYZ".into()],
                }),
            );
            doc.objects.insert(
                outlines_id,
                Object::Dictionary(dictionary! {
                    "Type" => "Outlines", "First" => first, "Last" => second, "Count" => 3,
                }),
            );
            catalog.set("Outlines", outlines_id);
        }
        let catalog_id = doc.add_object(catalog);
        doc.trailer.set("Root", catalog_id);

        let mut out = Vec::new();
        doc.save_to(&mut out).expect("saves");
        out
    }

    #[test]
    fn a_document_says_how_many_pages_it_has() {
        let doc = Document::open(&pdf_with(&["one", "two", "three"], false)).expect("opens");
        assert_eq!(doc.page_count(), 3);
    }

    #[test]
    fn something_that_is_not_a_pdf_is_refused_rather_than_opened() {
        let err = Document::open(b"this is a text file, not a PDF").unwrap_err();
        assert!(matches!(err, PdfError::Unreadable(_)));
        // And the message says what happened, because it reaches a reader.
        assert!(err.to_string().contains("could not be read as a PDF"));
    }

    #[test]
    fn a_document_with_no_contents_page_says_so_rather_than_failing() {
        // The distinction the whole outline API rests on: absent is an answer.
        let doc = Document::open(&pdf_with(&["only page"], false)).expect("opens");
        assert!(doc.outline().is_empty());
    }

    #[test]
    fn the_authors_own_headings_come_back_in_order_with_their_depth() {
        let doc = Document::open(&pdf_with(&["first", "second"], true)).expect("opens");
        let toc = doc.outline();
        let seen: Vec<(&str, usize)> =
            toc.iter().map(|e| (e.title.as_str(), e.depth)).collect();
        assert_eq!(
            seen,
            vec![("Introduction", 0), ("Background", 1), ("Method", 0)],
            "a child sits under its parent and the next sibling returns to the top level"
        );
    }

    #[test]
    fn a_heading_carries_the_page_it_jumps_to() {
        let doc = Document::open(&pdf_with(&["first", "second"], true)).expect("opens");
        let toc = doc.outline();
        assert_eq!(toc[0].page, Some(1), "one-based, so it reads as a page number");
        assert_eq!(toc[2].page, Some(2));
    }
}
