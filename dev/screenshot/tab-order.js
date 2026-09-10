// Does Tab walk the page the way the eye reads it?
//
// The focus RING probe answers whether you can see where you are. This answers
// whether where you are makes sense: a keyboard reader meets a surface in tab
// order, and when that order disagrees with the layout the page becomes a maze
// that looks perfectly ordinary in a screenshot. WCAG 2.4.3 is the same sentence
// with fewer words.
//
// WHAT IT DOES. Collect the focusable elements in the order Tab will reach them,
// then walk consecutive pairs and report the ones that go BACKWARDS in reading
// order: up a row, or leftwards within a row. Coordinates are included because
// the finding is a judgement a person has to make - "the sidebar comes after the
// content" can be correct - and a number is what lets them make it.
//
// WHAT IT IS NOT. Not a checker for the RIGHT order, which nothing can know: a
// two-column form, a toolbar on the right, a sidebar after the main region are
// all legitimate and all read as inversions here. It reports and does not judge,
// the way the axe table reports contrast it cannot decide.
//
// THE ORDER TAB ACTUALLY TAKES, which is not document order when a positive
// `tabindex` is in play: every positive index comes first, ascending, then
// everything at 0 in document order. A page that mixes them is usually already
// the defect, and modelling it wrongly here would hide exactly that page.
const SEL = "a[href], area[href], button, input, select, textarea, summary, [tabindex], [contenteditable='true']";

function tabbable() {
  const out = [];
  for (const el of document.querySelectorAll(SEL)) {
    if (el.disabled) continue;
    if (el.tabIndex < 0) continue;
    if (el.closest("[inert]") || el.closest("[aria-hidden='true']")) continue;
    const cs = getComputedStyle(el);
    if (cs.visibility === "hidden" || cs.display === "none") continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    out.push(el);
  }
  // Positive indices first, ascending and stable; then the zeros in document
  // order, which is what `out` already is.
  const positive = out.filter((e) => e.tabIndex > 0);
  const zero = out.filter((e) => e.tabIndex === 0);
  positive.sort((a, b) => a.tabIndex - b.tabIndex);
  return positive.concat(zero);
}

function label(el) {
  const id = el.id ? "#" + el.id : "";
  const cls = (el.getAttribute("class") || "").trim().split(/\s+/).filter(Boolean)[0];
  const text = (el.innerText || el.value || el.getAttribute("aria-label") || "").trim().slice(0, 24);
  return `${el.tagName.toLowerCase()}${id}${cls ? "." + cls : ""}${text ? `: "${text}"` : ""}`;
}

// A ROW IS NOT A PIXEL. Two controls on the same visual line rarely share a top
// edge - a 24px icon button beside a 40px field sits 8px lower - so "same row"
// is an overlap of their vertical extents rather than an equality, and a jump is
// only counted when the next control is CLEAR of the previous one's band.
function overlapsVertically(a, b) {
  return a.top < b.bottom && b.top < a.bottom;
}

const els = tabbable();
const out = [];
for (let i = 1; i < els.length; i++) {
  const prev = els[i - 1].getBoundingClientRect();
  const next = els[i].getBoundingClientRect();
  if (overlapsVertically(prev, next)) {
    // Same band: going right is forward, going left is back. The threshold is
    // the previous control's own width, so a two-column form whose second column
    // starts left of the first column's right edge is not reported.
    if (next.right <= prev.left) {
      out.push(
        `${label(els[i - 1])} → ${label(els[i])}: tab goes left ` +
          `${Math.round(prev.left - next.right)}px on the same line`,
      );
    }
    continue;
  }
  if (next.bottom <= prev.top) {
    out.push(
      `${label(els[i - 1])} → ${label(els[i])}: tab goes up ` +
        `${Math.round(prev.top - next.bottom)}px`,
    );
  }
}

// A PAGE WITH NOTHING TO TAB THROUGH is not an ordered one, and saying nothing
// about it would count it as coverage - the same vacuity the focus-ring probe
// learnt to name.
if (els.length === 0) {
  out.push("nothing on this page can be tabbed to");
} else if (els.length === 1) {
  out.push("one control on this page, so there is no order to disagree with");
}

return JSON.stringify(out);
