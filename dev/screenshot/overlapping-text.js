// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Text painted over text, which every other probe here is blind to.
//
// `clipped-text.js` asks whether an element outgrew its own box. Two elements can
// each fit their boxes perfectly and still be drawn on top of each other, and that
// is what a starved grid column does: the squeezed item overflows sideways into
// its neighbour and both are rendered, interleaved and unreadable.
//
// Found on the privacy page at 720 on 5 September, in German and in English: the
// scope of a grant ("deine Dateien") painted across its provenance ("bei der
// Installation angemeldet"), eight such pairs, overlaps up to 123x18px. The
// clipping sweep called that page clean, correctly - nothing had outgrown its box.
//
//   dev/screenshot/shoot.sh "http://localhost:5173/privacy?locale=de" \
//     /tmp/x.png dev/screenshot/overlapping-text.js 720 900
//
//   ["\"deine Dateien\" over \"bei der Installation a\" (27x18px)", ...]
//
// Leaves only, and boxes over 4px each way, so a decorative sliver behind a label
// is not reported. The 3px tolerance is there because adjacent boxes routinely
// share an edge; a real overprint is tens of pixels.
//
// IT COMPARES WHAT IS PAINTED, not what the layout box claims, and the difference
// is the whole reliability of this probe. A scroll container clips its children:
// the calendar's hour labels for the hours you have scrolled past still have boxes
// where they always were, well above the container, geometrically inside the app
// header. Comparing raw rects reported the header title "over" 06:00 on 5
// September and sent me looking for a layout bug in a calendar that was drawing
// correctly - twice, on the committed tree and on my own change, which is also
// how I learned it was not mine. So each rect is first intersected with every
// ancestor that clips (`overflow` other than `visible`); an element clipped to
// nothing is not on screen and is dropped before any comparison.
const clipped = (el) => {
  const r = el.getBoundingClientRect();
  let box = { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
  for (let p = el.parentElement; p; p = p.parentElement) {
    const s = getComputedStyle(p);
    if (s.overflowX === "visible" && s.overflowY === "visible") continue;
    const c = p.getBoundingClientRect();
    if (s.overflowX !== "visible") {
      box.left = Math.max(box.left, c.left);
      box.right = Math.min(box.right, c.right);
    }
    if (s.overflowY !== "visible") {
      box.top = Math.max(box.top, c.top);
      box.bottom = Math.min(box.bottom, c.bottom);
    }
  }
  box.width = box.right - box.left;
  box.height = box.bottom - box.top;
  return box;
};
// Leaf text nodes whose painted boxes intersect. Text over text is invisible to a
// clipping probe: both elements fit their own boxes perfectly.
// AND A CARD IN FRONT IS NOT AN OVERPRINT. A toast stack draws the deck: the
// rear notice keeps its box exactly where the front one is, and every pixel of
// it is behind an opaque card. Nothing is illegible, and the shell reported it
// at all three widths as "The focus mode you had" over "Could not read your
// Qu" - 305x21px, the whole line, and a picture of it shows one readable notice
// with a sliver of the next below.
//
// The rule that separates it from the real thing: TWO TEXTS ONLY OVERPRINT IF
// NOTHING OPAQUE SITS BETWEEN THEM. On the privacy page the scope and the
// provenance were bare spans in one grid, with no painted box between either of
// them and their common ancestor, so they interleaved on the page's own
// background. In a stack each notice sits on its own filled card, and the card
// is what the reader sees.
const opaqueBetween = (el, stop) => {
  for (let p = el; p && p !== stop; p = p.parentElement) {
    const s = getComputedStyle(p);
    if (s.backgroundImage !== "none") return true;
    const m = s.backgroundColor.match(/rgba?\(([^)]+)\)/);
    if (!m) continue;
    const parts = m[1].split(",").map((v) => parseFloat(v));
    // Three components means `rgb(...)`, which is opaque; four carries the
    // alpha. Half is the line: a wash over a neighbour still lets it through,
    // a card does not.
    const alpha = parts.length < 4 ? 1 : parts[3];
    if (alpha > 0.5) return true;
  }
  return false;
};
const commonAncestor = (a, b) => {
  const seen = new Set();
  for (let p = a; p; p = p.parentElement) seen.add(p);
  for (let p = b; p; p = p.parentElement) if (seen.has(p)) return p;
  return null;
};
const leaves = [];
for (const el of document.querySelectorAll("body *")) {
  if (el.children.length) continue;
  if (!(el.textContent || "").trim()) continue;
  const s = getComputedStyle(el);
  if (s.display === "none" || s.visibility === "hidden" || s.opacity === "0") continue;
  const box = clipped(el);
  if (box.width > 4 && box.height > 4) leaves.push({ el, box });
}
const out = [];
for (let i = 0; i < leaves.length; i++) {
  for (let j = i + 1; j < leaves.length; j++) {
    const a = leaves[i].box;
    const b = leaves[j].box;
    const ox = Math.min(a.right, b.right) - Math.max(a.left, b.left);
    const oy = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
    if (ox > 3 && oy > 3) {
      const stop = commonAncestor(leaves[i].el, leaves[j].el);
      if (opaqueBetween(leaves[i].el, stop) || opaqueBetween(leaves[j].el, stop)) continue;
      out.push(
        `"${leaves[i].el.textContent.trim().slice(0, 22)}" over "${leaves[j].el.textContent.trim().slice(0, 22)}" (${Math.round(ox)}x${Math.round(oy)}px)`,
      );
    }
  }
}
return JSON.stringify(out.slice(0, 8));
