// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Text a box is too small to hold, as a list rather than an impression.
//
// Pass it as the inject argument to `shoot.sh` and read the `inject result:`
// line; the screenshot is still written, so a hit can be looked at as well as
// counted:
//
//   dev/screenshot/shoot.sh "http://localhost:1421/topbar?locale=de" \
//     /tmp/x.png dev/screenshot/clipped-text.js
//
// WHY THIS AND NOT THE EYE. The defects it is looking for are the ones a fixed
// pixel width takes when a longer language goes through it - `Vertrauensstufe`
// past a column sized for `Trust level` - and they are easy to miss in a
// screenshot because a clipped label still looks like a label. German is where
// they surface; the harness renders any locale via `?locale=de`.
//
// It reports LEAF elements only. A container legitimately scrolls its children,
// so counting it would bury the real hits under every scroll region on the page.
//
// BOTH AXES, since 5 September. It measured width alone for its first month, on
// the reasoning that a long word running past a fixed column is what a second
// language does. That is one of two shapes: the calendar's week grid at 1920
// renders "Standup" and "Call with New York" cut off at the BOTTOM of their
// blocks, because a fifteen-minute event is shorter than its own label, and the
// horizontal sweep called that page clean. A label cut across the middle reads
// exactly as unreadable whichever direction the box ran out in.
//
// CONTROL, because a probe that returns nothing is indistinguishable from a
// probe that does not work - and mine returned nothing on three pages before I
// checked it. It is a FILE now rather than a sentence here, since a fixture
// described in a comment is one nobody runs:
//
//   dev/screenshot/shoot.sh "file://$PWD/dev/screenshot/clipped-text-control.html" \
//     /tmp/x.png dev/screenshot/clipped-text.js 800 400
//
//   ['div.: "Vertrauensstufe verschlagwortet" wide-cut 60<186',
//    'div.: "A line taller than its box" tall-cut 14<19']
//
// Four boxes: one too narrow, one too short, one screen-reader-hidden and one
// that fits. The last two must NOT appear. So an empty result means the page is
// clean, not that the probe is asleep.
const out = [];
for (const el of document.querySelectorAll("body *")) {
  const s = getComputedStyle(el);
  if (s.display === "none" || s.visibility === "hidden") continue;
  // A VISUALLY-HIDDEN LABEL IS NOT A CLIPPED ONE. The screen-reader pattern is a
  // 1x1 box with `overflow: hidden` and `clip: rect(0,0,0,0)`, so it overflows by
  // construction - on both axes, which is how adding the vertical check turned
  // every `sr-only` in the tree into a finding. Settings reported `div. "Settings"
  // wide 1<62, tall 1<24` the moment the second axis went in, and that element is
  // doing exactly what it should.
  //
  // Recognised by shape rather than class name, since `.sr-only` is one spelling
  // of it: a box no larger than a pixel each way is not showing anybody anything.
  if (el.clientWidth <= 1 && el.clientHeight <= 1) continue;
  if (el.children.length > 0) continue;
  const t = (el.textContent || "").trim();
  if (!t) continue;
  const cls = (el.className || "").toString().split(" ")[0];
  const where = `${el.tagName.toLowerCase()}.${cls}: "${t.slice(0, 40)}"`;
  // DECLARED TRUNCATION IS NOT THE SAME AS A CUT, and the probe could not tell
  // them apart. A link-card description with `text-overflow: ellipsis` and
  // `white-space: nowrap` is a design saying "this line ends in a … when it runs
  // out"; a label without them is a design that did not know it would run out.
  // Both lose text and only the second is a surprise, so they are LABELLED rather
  // than merged - and neither is dropped, because an ellipsis still withholds
  // words unless something else offers them, which is a judgement for a reader
  // and not for this.
  const declared = s.textOverflow === "ellipsis" && s.whiteSpace.startsWith("nowrap");
  const kind = declared ? "ellipsed" : "cut";
  // Named per axis rather than merged, because the fixes are different: a wide
  // one wants a floor, an ellipsis or a shorter string, a tall one wants room or
  // fewer lines.
  if (el.scrollWidth > el.clientWidth + 1) {
    out.push(`${where} wide-${kind} ${el.clientWidth}<${el.scrollWidth}`);
  }
  if (el.scrollHeight > el.clientHeight + 1) {
    out.push(`${where} tall-${kind} ${el.clientHeight}<${el.scrollHeight}`);
  }
}
return out.slice(0, 12);
