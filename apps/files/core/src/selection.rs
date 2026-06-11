//! The file-browser selection + keyboard-cursor model (FM-R1).
//!
//! A pure, index-based model over a listing of `count` entries: the host maps
//! indices to its `FileEntry` rows, so the same model drives the list, grid,
//! Miller and dual-pane views. It implements the conventional desktop selection
//! grammar - plain click, ctrl-toggle, shift-range, and arrow / shift-arrow
//! cursor movement - with an anchor for ranges and a cursor for keyboard focus.
//! Cursor movement takes a signed `delta`, so a host passes ±1 for a list or
//! ±columns for a grid, and `Home`/`End` are just a large delta (saturating).

use std::collections::BTreeSet;

/// The selection and keyboard-cursor state over a listing of `count` entries.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    count: usize,
    selected: BTreeSet<usize>,
    /// The fixed end of a shift-range (set by the last plain click / cursor move).
    anchor: Option<usize>,
    /// The keyboard-focused index.
    cursor: Option<usize>,
}

impl Selection {
    /// A fresh selection over `count` entries (nothing selected, no cursor).
    pub fn new(count: usize) -> Self {
        Self {
            count,
            ..Default::default()
        }
    }

    /// Re-base onto a freshly listed directory of `count` entries: the previous
    /// indices no longer map to the same files, so the selection, anchor and
    /// cursor are cleared. The host calls this on every navigation / re-list.
    pub fn set_count(&mut self, count: usize) {
        self.count = count;
        self.selected.clear();
        self.anchor = None;
        self.cursor = None;
    }

    /// The number of entries the model is over.
    pub fn count(&self) -> usize {
        self.count
    }

    /// Whether entry `i` is selected.
    pub fn is_selected(&self, i: usize) -> bool {
        self.selected.contains(&i)
    }

    /// The selected indices, ascending.
    pub fn selected(&self) -> Vec<usize> {
        self.selected.iter().copied().collect()
    }

    /// The keyboard-focused index, if any.
    pub fn cursor(&self) -> Option<usize> {
        self.cursor
    }

    /// Plain click: select only `i`, and make it the anchor and cursor. Out-of-
    /// range indices are ignored.
    pub fn click(&mut self, i: usize) {
        if i >= self.count {
            return;
        }
        self.selected.clear();
        self.selected.insert(i);
        self.anchor = Some(i);
        self.cursor = Some(i);
    }

    /// Ctrl-click: toggle `i` in the selection (keeping the rest), and make it the
    /// anchor and cursor so a following shift-range starts here.
    pub fn toggle(&mut self, i: usize) {
        if i >= self.count {
            return;
        }
        if !self.selected.insert(i) {
            self.selected.remove(&i);
        }
        self.anchor = Some(i);
        self.cursor = Some(i);
    }

    /// Shift-click: replace the selection with the contiguous range from the
    /// anchor to `i` (inclusive). With no anchor yet, `i` becomes the anchor (a
    /// single-item range). The anchor is preserved, so successive shift-clicks
    /// re-range from the same fixed end; the cursor follows `i`.
    pub fn range_to(&mut self, i: usize) {
        if i >= self.count {
            return;
        }
        let anchor = self.anchor.unwrap_or(i);
        self.anchor = Some(anchor);
        self.cursor = Some(i);
        self.select_range(anchor, i);
    }

    /// Move the keyboard cursor by `delta` (clamped to the listing). With
    /// `extend`, grow the shift-range from the anchor to the new cursor; without
    /// it, collapse to a single selection at the new cursor and re-anchor there.
    /// `delta` is `isize`, so a host passes ±1 (list), ±columns (grid), or
    /// `isize::MIN`/`MAX` for Home/End.
    pub fn move_cursor(&mut self, delta: isize, extend: bool) {
        if self.count == 0 {
            return;
        }
        let max = self.count as isize - 1;
        let from = self.cursor.unwrap_or(0) as isize;
        let next = from.saturating_add(delta).clamp(0, max) as usize;
        self.cursor = Some(next);
        if extend {
            let anchor = self.anchor.unwrap_or(next);
            self.anchor = Some(anchor);
            self.select_range(anchor, next);
        } else {
            self.selected.clear();
            self.selected.insert(next);
            self.anchor = Some(next);
        }
    }

    /// Select every entry; anchor at 0, cursor at the last.
    pub fn select_all(&mut self) {
        self.selected = (0..self.count).collect();
        if self.count > 0 {
            self.anchor = Some(0);
            self.cursor = Some(self.count - 1);
        }
    }

    /// Clear the selection and the anchor (the cursor stays, so keyboard focus is
    /// not lost on Escape).
    pub fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    /// Replace the selection with the inclusive index range `[min(a,b), max(a,b)]`.
    fn select_range(&mut self, a: usize, b: usize) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        self.selected = (lo..=hi).filter(|i| *i < self.count).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_selects_only_one_and_sets_anchor_cursor() {
        let mut s = Selection::new(5);
        s.click(2);
        assert_eq!(s.selected(), vec![2]);
        assert_eq!(s.cursor(), Some(2));
        s.click(4);
        assert_eq!(s.selected(), vec![4], "a new click replaces the selection");
        // Out-of-range is ignored.
        s.click(9);
        assert_eq!(s.selected(), vec![4]);
    }

    #[test]
    fn ctrl_toggle_adds_and_removes_keeping_the_rest() {
        let mut s = Selection::new(5);
        s.click(1);
        s.toggle(3);
        s.toggle(4);
        assert_eq!(s.selected(), vec![1, 3, 4]);
        s.toggle(3); // toggle off
        assert_eq!(s.selected(), vec![1, 4]);
    }

    #[test]
    fn shift_range_selects_from_the_anchor_inclusive_both_directions() {
        let mut s = Selection::new(8);
        s.click(2); // anchor 2
        s.range_to(5);
        assert_eq!(s.selected(), vec![2, 3, 4, 5]);
        // Re-range from the SAME anchor, downward.
        s.range_to(0);
        assert_eq!(s.selected(), vec![0, 1, 2]);
    }

    #[test]
    fn arrow_moves_cursor_and_single_selects_clamped() {
        let mut s = Selection::new(3);
        s.move_cursor(1, false);
        assert_eq!((s.cursor(), s.selected()), (Some(1), vec![1]));
        s.move_cursor(1, false);
        s.move_cursor(1, false); // clamps at 2
        assert_eq!((s.cursor(), s.selected()), (Some(2), vec![2]));
        // Home/End via a saturating large delta.
        s.move_cursor(isize::MIN, false);
        assert_eq!(s.cursor(), Some(0));
        s.move_cursor(isize::MAX, false);
        assert_eq!(s.cursor(), Some(2));
    }

    #[test]
    fn shift_arrow_extends_the_range_from_the_anchor() {
        let mut s = Selection::new(6);
        s.click(1); // anchor + cursor 1
        s.move_cursor(1, true); // -> 2
        s.move_cursor(1, true); // -> 3
        assert_eq!(s.selected(), vec![1, 2, 3]);
        assert_eq!(s.cursor(), Some(3));
        // Reverse past the anchor: the range flips around the fixed anchor.
        s.move_cursor(-3, true); // -> 0
        assert_eq!(s.selected(), vec![0, 1]);
    }

    #[test]
    fn select_all_and_clear() {
        let mut s = Selection::new(4);
        s.select_all();
        assert_eq!(s.selected(), vec![0, 1, 2, 3]);
        assert_eq!(s.cursor(), Some(3));
        s.clear();
        assert!(s.selected().is_empty());
        assert_eq!(s.cursor(), Some(3), "clear keeps keyboard focus");
    }

    #[test]
    fn set_count_rebases_and_empty_listing_is_inert() {
        let mut s = Selection::new(4);
        s.click(2);
        s.set_count(10); // navigated to a new directory
        assert!(s.selected().is_empty() && s.cursor().is_none());
        let mut empty = Selection::new(0);
        empty.move_cursor(1, false);
        empty.click(0);
        empty.select_all();
        assert!(empty.selected().is_empty() && empty.cursor().is_none());
    }
}
