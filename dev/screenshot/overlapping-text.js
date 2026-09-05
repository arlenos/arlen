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
// Leaf text nodes whose painted boxes intersect. Text over text is invisible to a
// clipping probe: both elements fit their own boxes perfectly.
const leaves = [...document.querySelectorAll("body *")].filter((el) => {
  if (el.children.length) return false;
  if (!(el.textContent || "").trim()) return false;
  const s = getComputedStyle(el);
  if (s.display === "none" || s.visibility === "hidden" || s.opacity === "0") return false;
  const r = el.getBoundingClientRect();
  return r.width > 4 && r.height > 4;
});
const out = [];
for (let i = 0; i < leaves.length; i++) {
  for (let j = i + 1; j < leaves.length; j++) {
    const a = leaves[i].getBoundingClientRect();
    const b = leaves[j].getBoundingClientRect();
    const ox = Math.min(a.right, b.right) - Math.max(a.left, b.left);
    const oy = Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top);
    if (ox > 3 && oy > 3) {
      out.push(
        `"${leaves[i].textContent.trim().slice(0, 22)}" over "${leaves[j].textContent.trim().slice(0, 22)}" (${Math.round(ox)}x${Math.round(oy)}px)`,
      );
    }
  }
}
return JSON.stringify(out.slice(0, 8));
