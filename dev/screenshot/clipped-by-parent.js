// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Text an ANCESTOR cuts off, which the other two probes are blind to by
// construction.
//
// `clipped-text.js` asks whether an element outgrew its own box.
// `overlapping-text.js` asks whether two elements share a painted area. Neither
// sees the third shape: an element that fits its box perfectly, sitting inside a
// container with `overflow: hidden`, positioned past that container's edge. The
// text is simply not there, and every measurement of the element itself says it is
// fine.
//
// Found by eye on 5 September while fixing the privacy page: a row grew wider than
// its panel and the "Entfernen" action was cut to its first letter by the panel
// edge. The clipping sweep said the page was clean and was right about the
// question it asks.
//
//   dev/screenshot/shoot.sh "http://localhost:5173/privacy?locale=de" \
//     /tmp/x.png dev/screenshot/clipped-by-parent.js 720 900
//
//   ['span.remove: "Entfernen" cut 34px past .panel']
//
// ONLY `hidden` AND `clip` ANCESTORS. A scroll container holds content beyond its
// box on purpose and the reader can reach it, so counting those would report every
// long list in the tree - and a probe that reports the ordinary case is one nobody
// reads. That does leave a real gap: a scrollable container with no visible way to
// scroll hides text too, and telling those apart needs more than geometry.
const out = [];
const clips = (v) => v === "hidden" || v === "clip";
for (const el of document.querySelectorAll("body *")) {
  if (el.children.length) continue;
  const t = (el.textContent || "").trim();
  if (!t) continue;
  const s = getComputedStyle(el);
  if (s.display === "none" || s.visibility === "hidden") continue;
  const r = el.getBoundingClientRect();
  if (r.width <= 1 || r.height <= 1) continue;
  for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
    const ps = getComputedStyle(p);
    const pr = p.getBoundingClientRect();
    // PER AXIS, because a box clips per axis. The first cut of this took the max
    // of all four sides against any clipping parent, so every element below the
    // fold of a vertically-clipped container read as cut - which on the privacy
    // page was most of the page, and a probe that reports most of the page reports
    // nothing. An element is only cut on the axis its ancestor actually clips.
    // SIDEWAYS ONLY, and the vertical axis is left alone deliberately. Content
    // below the fold of a clipped wrapper is the ordinary case - some ancestor
    // scrolls and the reader reaches it - so counting it reported most of the
    // privacy page. Telling an unreachable vertical cut from a scrollable one
    // needs to know which ancestor scrolls and whether it can, which is more than
    // geometry, so this answers the question it can: text pushed out the SIDE of a
    // box that will not give it back.
    const past = clips(ps.overflowX)
      ? Math.max(pr.left - r.left, r.right - pr.right)
      : 0;
    if (past > 3) {
      const cls = (p.className || "").toString().split(" ")[0] || p.tagName.toLowerCase();
      out.push(`${el.tagName.toLowerCase()}.${(el.className || "").toString().split(" ")[0]}: "${t.slice(0, 30)}" cut ${Math.round(past)}px sideways by .${cls}`);
      break;
    }
  }
}
return out.slice(0, 12);
