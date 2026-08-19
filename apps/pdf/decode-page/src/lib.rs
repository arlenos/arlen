// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only

//! Draw one page of a PDF, and refuse anything that would not fit on a screen.
//!
//! Split out of the reader as its own worker binary for the reason the viewer's
//! decoders are: this is the largest piece of C in the tree and it parses files
//! people are sent by strangers. A bug in it should cost a page, not the reader
//! and not the session, so it runs behind bwrap, seccomp and Landlock and hands
//! back nothing but pixels.
//!
//! The bounds here are the ones a malicious document reaches for. A PDF names
//! its page size in points and the renderer multiplies by the scale, so a page
//! declaring itself a kilometre wide is a memory bomb written in three numbers.
//! The size is therefore checked BEFORE the raster is allocated rather than
//! after, which is the difference between a refusal and an out-of-memory kill.

/// The most pixels one rendered page may cover.
///
/// Sixteen megapixels is a 4000x4000 page, past any paper size at a readable
/// zoom and far short of an allocation that hurts. Refused before allocating.
pub const MAX_PIXELS: u64 = 16 * 1024 * 1024;

/// The scale range a caller may ask for, in page points to pixels.
///
/// Clamped rather than rejected: a caller asking for a hundredfold zoom has a
/// bug, and refusing the page tells a reader nothing while a clamped page is
/// still the page.
pub const MIN_SCALE: f32 = 0.1;
/// See [`MIN_SCALE`].
pub const MAX_SCALE: f32 = 8.0;

/// One page, drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Raster {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA, four bytes a pixel.
    pub rgba: Vec<u8>,
}

impl Raster {
    /// The frame as it goes over the pipe: `RGBA`, width, height, then the body.
    ///
    /// A magic word and the dimensions first, so a reader that gets a truncated
    /// or foreign frame can say so rather than interpreting whatever arrived as
    /// pixels. Little-endian, both ends being the same machine by construction.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(12 + self.rgba.len());
        out.extend_from_slice(b"RGBA");
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.rgba);
        out
    }
}

/// One line of text on a page, and where it sits.
///
/// Per LINE rather than per glyph, because that is the granularity a reader
/// selects in: an overlay of one box per character makes a page of a thousand
/// invisible elements and selects like one too. Coordinates are in the same
/// pixel space as the rendered raster at the same scale, so a surface can lay
/// these straight over the canvas without a second transform to get wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    /// The line as written.
    pub text: String,
    /// Left edge, in pixels from the page's left.
    pub x: f32,
    /// Top edge, in pixels from the page's top.
    pub y: f32,
    /// Width in pixels.
    pub width: f32,
    /// Height in pixels.
    pub height: f32,
}

/// The text on page `page` (one-based) of `bytes`, with each line's box.
///
/// Empty is a real answer: a scanned page carries an image and no text, and
/// that is different from a failure.
///
/// # Errors
/// The same shapes as [`render_page`]: bytes that are not a PDF, or a page the
/// document does not have.
pub fn page_text_layer(bytes: &[u8], page: usize, scale: f32) -> Result<Vec<TextLine>, String> {
    use mupdf::{Document, TextPageFlags};

    let scale = if scale.is_finite() { scale.clamp(MIN_SCALE, MAX_SCALE) } else { 1.0 };
    let doc = Document::from_bytes(bytes, "application/pdf")
        .map_err(|e| format!("this file could not be read as a PDF: {e}"))?;
    let count = usize::try_from(doc.page_count().unwrap_or(0)).unwrap_or(0);
    if page == 0 || page > count {
        return Err(format!("this PDF has {count} pages, so there is no page {page}"));
    }
    let index = i32::try_from(page - 1).map_err(|_| format!("page {page} is out of range"))?;
    let loaded = doc.load_page(index).map_err(|e| format!("page {page} would not load: {e}"))?;
    let text = loaded
        .to_text_page(TextPageFlags::empty())
        .map_err(|e| format!("page {page} has no readable text layer: {e}"))?;

    let mut out = Vec::new();
    for block in text.blocks() {
        for line in block.lines() {
            let mut s = String::new();
            for ch in line.chars() {
                if let Some(c) = ch.char() {
                    s.push(c);
                }
            }
            // A line that is only whitespace positions nothing a reader would
            // select, and a box over it is a box that swallows clicks.
            if s.trim().is_empty() {
                continue;
            }
            let b = line.bounds();
            out.push(TextLine {
                text: s,
                x: b.x0 * scale,
                y: b.y0 * scale,
                width: (b.x1 - b.x0) * scale,
                height: (b.y1 - b.y0) * scale,
            });
        }
    }
    Ok(out)
}

/// Render page `page` (one-based) of `bytes` at `scale`.
///
/// # Errors
/// A sentence naming what went wrong: bytes that are not a PDF, a page the
/// document does not have, or a page whose raster would exceed [`MAX_PIXELS`].
pub fn render_page(bytes: &[u8], page: usize, scale: f32) -> Result<Raster, String> {
    use mupdf::{Colorspace, Document, Matrix};

    let scale = if scale.is_finite() { scale.clamp(MIN_SCALE, MAX_SCALE) } else { 1.0 };
    let doc = Document::from_bytes(bytes, "application/pdf")
        .map_err(|e| format!("this file could not be read as a PDF: {e}"))?;
    let count = doc.page_count().map_err(|e| format!("its pages could not be counted: {e}"))?;
    let count = usize::try_from(count).unwrap_or(0);
    if page == 0 || page > count {
        return Err(format!("this PDF has {count} pages, so there is no page {page}"));
    }
    let index = i32::try_from(page - 1).map_err(|_| format!("page {page} is out of range"))?;
    let loaded = doc.load_page(index).map_err(|e| format!("page {page} would not load: {e}"))?;

    // Checked against the page's OWN declared size before anything is drawn: the
    // document controls these numbers, and multiplying them by the scale is
    // exactly where a hostile file turns three integers into an allocation.
    let bounds = loaded.bounds().map_err(|e| format!("page {page} has no usable size: {e}"))?;
    let w = f64::from((bounds.x1 - bounds.x0) * scale).ceil().max(1.0);
    let h = f64::from((bounds.y1 - bounds.y0) * scale).ceil().max(1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let pixels = (w as u64).saturating_mul(h as u64);
    if pixels > MAX_PIXELS {
        return Err(format!(
            "page {page} would be {w} by {h} pixels at this zoom, past the {MAX_PIXELS} the reader will draw"
        ));
    }

    let pixmap = loaded
        // OPAQUE, not alpha. A page is paper: with alpha the pixmap starts
        // transparent and only the marks are painted, so a viewer compositing it
        // over its own background gets something that looks right until the
        // background is not white. It also cost an hour here - a fully
        // transparent raster reads as "black" to a naive pixel check, so a page
        // that drew NOTHING measured as a page covered in ink.
        .to_pixmap(&Matrix::new_scale(scale, scale), &Colorspace::device_rgb(), false, false)
        .map_err(|e| format!("page {page} could not be drawn: {e}"))?;
    let (width, height) = (pixmap.width(), pixmap.height());
    let samples = pixmap.samples();
    // Asked rather than assumed: an RGB pixmap and an RGBA one differ by a byte
    // a pixel, and reading one as the other shears the image into diagonal
    // stripes - a wrong picture rather than an error, which is the worse failure.
    let rgba = match pixmap.n() {
        4 => samples.to_vec(),
        3 => samples.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 0xFF]).collect(),
        other => return Err(format!("page {page} came back with {other} bytes a pixel")),
    };
    let expected = (width as usize).saturating_mul(height as usize).saturating_mul(4);
    if rgba.len() != expected {
        return Err(format!("page {page} came back {} bytes, not the {expected} its size needs", rgba.len()));
    }
    Ok(Raster { width, height, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest real PDF that has a page: one page, no content stream.
    fn one_page_pdf() -> Vec<u8> {
        let body = concat!(
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n",
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n",
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] >>\nendobj\n",
        );
        format!("%PDF-1.5\n{body}trailer\n<< /Size 4 /Root 1 0 R >>\n%%EOF\n").into_bytes()
    }

    /// A page with one line of text in a font it does NOT embed - the ordinary
    /// case, and the one that needs the base-14 set to draw at all.
    fn text_page_pdf() -> Vec<u8> {
        let stream = b"BT /F1 24 Tf 20 40 Td (Hello) Tj ET";
        let mut out = Vec::from(&b"%PDF-1.5\n"[..]);
        let mut offsets = Vec::new();
        let add = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: Vec<u8>| {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
            out.extend_from_slice(&body);
            out.extend_from_slice(b"\nendobj\n");
        };
        add(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
        add(&mut out, &mut offsets, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec());
        add(&mut out, &mut offsets,
            b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> /MediaBox [0 0 200 100] >>".to_vec());
        let mut content = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        content.extend_from_slice(stream);
        content.extend_from_slice(b"\nendstream");
        add(&mut out, &mut offsets, content);
        add(&mut out, &mut offsets,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
        let startxref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
        for off in &offsets {
            out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{startxref}\n%%EOF\n",
                    offsets.len() + 1).as_bytes());
        out
    }

    #[test]
    fn a_page_that_has_text_on_it_comes_back_with_ink_on_it() {
        // THE case, and the one that was missing when this crate first shipped:
        // `base14-fonts` was off, so a document naming Helvetica got a page with
        // no glyphs on it - clean white paper, no error, no warning. Every other
        // test here passed. Only counting dark pixels says otherwise.
        let out = render_page(&text_page_pdf(), 1, 1.0).expect("renders");
        let dark = out.rgba.chunks_exact(4).filter(|p| p[0] < 128).count();
        assert!(dark > 20, "a page with text on it drew {dark} dark pixels");
    }

    #[test]
    fn the_text_layer_says_what_the_line_reads_and_where_it_sits() {
        let lines = page_text_layer(&text_page_pdf(), 1, 1.0).expect("reads");
        assert_eq!(lines.len(), 1, "one line of text, one entry");
        assert!(lines[0].text.contains("Hello"), "got {:?}", lines[0].text);
        // Inside the 200x100 page it was written on, and not a zero-size box:
        // a box with no area selects nothing, which is the failure that looks
        // like a working text layer.
        assert!(lines[0].width > 1.0 && lines[0].height > 1.0, "got {:?}", lines[0]);
        assert!(lines[0].x >= 0.0 && lines[0].x < 200.0);
        assert!(lines[0].y >= 0.0 && lines[0].y < 100.0);
    }

    #[test]
    fn the_text_layer_scales_with_the_page_it_is_laid_over() {
        // It has to land on the raster rendered at the SAME scale, so both move
        // together or the boxes drift off the words at every zoom but one.
        let one = page_text_layer(&text_page_pdf(), 1, 1.0).expect("reads");
        let two = page_text_layer(&text_page_pdf(), 1, 2.0).expect("reads");
        assert!((two[0].x - one[0].x * 2.0).abs() < 0.01, "{:?} vs {:?}", one[0], two[0]);
        assert!((two[0].width - one[0].width * 2.0).abs() < 0.01);
    }

    #[test]
    fn a_page_with_no_text_has_an_empty_layer_rather_than_a_failure() {
        // What a scan looks like. Empty and broken must not read the same.
        assert_eq!(page_text_layer(&one_page_pdf(), 1, 1.0).expect("reads"), Vec::new());
    }

    #[test]
    fn a_page_comes_back_as_pixels_of_the_size_the_document_asked_for() {
        let out = render_page(&one_page_pdf(), 1, 1.0).expect("renders");
        assert_eq!((out.width, out.height), (200, 100));
        assert_eq!(out.rgba.len(), 200 * 100 * 4, "four bytes a pixel, no shear");
    }

    #[test]
    fn the_scale_is_the_scale() {
        let out = render_page(&one_page_pdf(), 1, 2.0).expect("renders");
        assert_eq!((out.width, out.height), (400, 200));
    }

    #[test]
    fn a_page_that_is_not_there_is_named_rather_than_drawn() {
        let err = render_page(&one_page_pdf(), 2, 1.0).unwrap_err();
        assert!(err.contains("no page 2"), "got {err}");
        assert!(render_page(&one_page_pdf(), 0, 1.0).is_err(), "one-based, so zero is meaningless");
    }

    #[test]
    fn something_that_is_not_a_pdf_is_refused() {
        assert!(render_page(b"not a pdf at all", 1, 1.0).is_err());
    }

    #[test]
    fn an_absurd_zoom_is_clamped_rather_than_allocated() {
        // The bound that matters: a caller bug must not become an allocation.
        let out = render_page(&one_page_pdf(), 1, 1e9).expect("clamped and drawn");
        assert!(u64::from(out.width) * u64::from(out.height) <= MAX_PIXELS);
        let tiny = render_page(&one_page_pdf(), 1, -5.0).expect("clamped up");
        assert!(tiny.width >= 1 && tiny.height >= 1);
    }

    #[test]
    fn a_drawn_page_is_opaque_paper_rather_than_a_transparent_sheet() {
        // The case this exists for. A transparent raster looks like a working
        // render to anything that only checks the size, and it looks like a
        // blank page on screen - which is indistinguishable from a page that
        // failed to draw. Every pixel must carry full alpha.
        let out = render_page(&one_page_pdf(), 1, 1.0).expect("renders");
        assert!(
            out.rgba.chunks_exact(4).all(|p| p[3] == 0xFF),
            "a page with a transparent pixel is a page that drew nothing"
        );
    }

    #[test]
    fn the_frame_says_what_it_is_before_it_says_how_big() {
        let out = render_page(&one_page_pdf(), 1, 1.0).expect("renders");
        let frame = out.encode();
        assert_eq!(&frame[..4], b"RGBA");
        assert_eq!(u32::from_le_bytes(frame[4..8].try_into().unwrap()), out.width);
        assert_eq!(frame.len(), 12 + out.rgba.len());
    }
}
