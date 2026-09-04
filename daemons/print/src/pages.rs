//! What `1-5, 8` means, in one place.
//!
//! The page-range box is the only print option whose purpose is to print LESS
//! than the document, so a range nobody can read must not fall back to printing
//! everything: somebody who asked for one page of a two hundred page report and
//! got the report has had the opposite of what they asked for, on paper, with no
//! way to take it back. Every failure here is a refusal.

/// One inclusive page range.
pub type PageRange = (i32, i32);

/// Why a typed range was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeError {
    /// Nothing but whitespace.
    Empty,
    /// A part that is not a page number or a `from-to`.
    Unreadable(String),
    /// A range that ends before it starts, or a page zero.
    Backwards(String),
    /// A page past the end of the document.
    PastTheEnd { page: i32, pages: i32 },
}

impl RangeError {
    /// The sentence to show. Names the part that was wrong, because "invalid
    /// range" against a box holding `1-5, 8, 12-` tells nobody which bit to fix.
    pub fn message(&self) -> String {
        match self {
            RangeError::Empty => "No pages are selected, so there is nothing to print.".to_string(),
            RangeError::Unreadable(part) => {
                format!("{part} is not a page or a range of pages. Nothing was printed.")
            }
            RangeError::Backwards(part) => {
                format!("{part} ends before it starts. Nothing was printed.")
            }
            RangeError::PastTheEnd { page, pages } => {
                format!("Page {page} is past the end of a {pages} page document. Nothing was printed.")
            }
        }
    }
}

/// Parse a typed page range.
///
/// `pages` is the document's length when it is known, and `0` means it is not -
/// a compressed PDF whose page tree could not be counted. An unknown length
/// checks the shape and lets the printer decide what is past the end, which is
/// the honest split: this refuses what it can prove wrong and does not invent a
/// bound it does not have.
pub fn parse(text: &str, pages: i32) -> Result<Vec<PageRange>, RangeError> {
    let mut out = Vec::new();
    for part in text.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (from, to) = match part.split_once('-') {
            Some((a, b)) => {
                let a = a.trim();
                let b = b.trim();
                // An open end (`5-`) is a real thing to type and means "to the
                // end", which is only expressible when the length is known.
                let from = a
                    .parse::<i32>()
                    .map_err(|_| RangeError::Unreadable(part.to_string()))?;
                if b.is_empty() {
                    if pages <= 0 {
                        return Err(RangeError::Unreadable(part.to_string()));
                    }
                    (from, pages)
                } else {
                    (
                        from,
                        b.parse::<i32>()
                            .map_err(|_| RangeError::Unreadable(part.to_string()))?,
                    )
                }
            }
            None => {
                let n = part
                    .parse::<i32>()
                    .map_err(|_| RangeError::Unreadable(part.to_string()))?;
                (n, n)
            }
        };
        if from < 1 || to < from {
            return Err(RangeError::Backwards(part.to_string()));
        }
        if pages > 0 && to > pages {
            return Err(RangeError::PastTheEnd { page: to, pages });
        }
        out.push((from, to));
    }
    if out.is_empty() {
        return Err(RangeError::Empty);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ordinary_shapes_read() {
        assert_eq!(parse("1-5, 8", 12).unwrap(), vec![(1, 5), (8, 8)]);
        assert_eq!(parse("3", 12).unwrap(), vec![(3, 3)]);
        assert_eq!(parse(" 2 - 4 ,, 9 ", 12).unwrap(), vec![(2, 4), (9, 9)]);
    }

    #[test]
    fn an_open_end_runs_to_the_last_page() {
        assert_eq!(parse("9-", 12).unwrap(), vec![(9, 12)]);
    }

    #[test]
    fn an_open_end_with_no_known_length_is_refused_rather_than_guessed() {
        assert!(matches!(parse("9-", 0), Err(RangeError::Unreadable(_))));
    }

    #[test]
    fn nothing_selected_is_not_everything() {
        assert_eq!(parse("", 12), Err(RangeError::Empty));
        assert_eq!(parse("  , ", 12), Err(RangeError::Empty));
    }

    #[test]
    fn a_part_nobody_can_read_refuses_the_whole_job() {
        assert!(matches!(parse("1-5, eight", 12), Err(RangeError::Unreadable(p)) if p == "eight"));
        assert!(matches!(parse("1..5", 12), Err(RangeError::Unreadable(_))));
    }

    #[test]
    fn a_backwards_range_or_a_page_zero_is_refused() {
        assert!(matches!(parse("5-2", 12), Err(RangeError::Backwards(_))));
        assert!(matches!(parse("0", 12), Err(RangeError::Backwards(_))));
    }

    #[test]
    fn a_page_past_the_end_is_refused_when_the_end_is_known() {
        assert_eq!(
            parse("11-14", 12),
            Err(RangeError::PastTheEnd { page: 14, pages: 12 })
        );
        // and accepted when it is not, because nothing here knows better
        assert!(parse("11-14", 0).is_ok());
    }

    #[test]
    fn a_refusal_says_which_part_and_that_nothing_printed() {
        let m = parse("1-5, eight", 12).unwrap_err().message();
        assert!(m.contains("eight") && m.contains("Nothing was printed"), "{m}");
    }
}
