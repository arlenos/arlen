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
const clipper = (el) => {
  const s = getComputedStyle(el);
  return s.overflowX === "hidden" || s.overflowX === "clip" ||
         s.overflowY === "hidden" || s.overflowY === "clip";
};
for (const el of document.querySelectorAll("body *")) {
  if (el.children.length) continue;
  const t = (el.textContent || "").trim();
  if (!t) continue;
  const s = getComputedStyle(el);
  if (s.display === "none" || s.visibility === "hidden") continue;
  const r = el.getBoundingClientRect();
  if (r.width <= 1 || r.height <= 1) continue;
  for (let p = el.parentElement; p && p !== document.body; p = p.parentElement) {
    if (!clipper(p)) continue;
    const pr = p.getBoundingClientRect();
    // How far the text sticks out of the box that will cut it. A pixel or two is
    // a rounded corner or a subpixel edge; a cut somebody notices is wider.
    const past = Math.max(pr.left - r.left, r.right - pr.right, pr.top - r.top, r.bottom - pr.bottom);
    if (past > 3) {
      const cls = (p.className || "").toString().split(" ")[0] || p.tagName.toLowerCase();
      out.push(`${el.tagName.toLowerCase()}.${(el.className || "").toString().split(" ")[0]}: "${t.slice(0, 30)}" cut ${Math.round(past)}px past .${cls}`);
      break;
    }
  }
}
return out.slice(0, 12);
