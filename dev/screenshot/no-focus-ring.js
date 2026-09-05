// A control that takes keyboard focus and does not look any different for it.
//
// The eye has one job here and it is not decoration: on a keyboard, the focus
// ring IS the pointer. A page whose buttons all take focus silently is a page
// where the only way to know where you are is to press the thing and find out.
// WCAG 2.4.7 says the same in fewer words.
//
// Nothing in this tree measured it. The kit's axe run cannot: axe has no
// focus-appearance rule, because deciding whether a change is VISIBLE needs a
// render and axe's own gate runs under jsdom. So this is a render probe.
//
// WHAT IT DOES. For every element that can take focus, snapshot the computed
// properties a ring is ever built from - on the element, on its pseudo-elements
// and on three ancestors, because `:focus-within` on a wrapper is as good a ring
// as `:focus` on the control - then focus it and snapshot again. Nothing changed
// anywhere means nothing was drawn.
//
// WHY ANCESTORS AND PSEUDOS RATHER THAN JUST `outline`. The three ring idioms in
// this tree are `outline`, an inset `box-shadow`, and a wrapper lighting up
// under `:focus-within`. A probe that only read `outline` would report the kit's
// search field, which is the false positive that ends a sweep's life.
//
// WHAT IT DELIBERATELY DOES NOT DO. It does not judge whether the ring is
// CONTRAST-ENOUGH or THICK-ENOUGH (WCAG 2.4.11/2.4.13). That is a measurement of
// two colours against a background this probe would have to guess at, and a
// guess that produces numbers is worse than no numbers. Absent is absent, and
// that is the finding worth having first.
// A CHANGED PROPERTY IS NOT A DRAWN ONE, which is the trap this probe fell into
// first. WebKit's own stylesheet moves `outline-offset` to -2px on a focused
// text field, and it does that whether or not there is an outline to offset -
// so a field whose ring had been deleted still "changed" and the probe called it
// ringed. Every property below is therefore reduced to what it PAINTS before it
// is compared: an outline with `style: none` is the token `none` however wide or
// far offset it is, and a pseudo-element with no `content` is `absent` whatever
// else it declares.
const SIDES = ["Top", "Right", "Bottom", "Left"];
const PLAIN = ["boxShadow", "backgroundColor", "color", "opacity", "filter",
               "textDecorationLine", "textDecorationColor"];
const PSEUDO = [null, "::before", "::after"];

function painted(cs) {
  const parts = [];
  const drawn = cs.outlineStyle !== "none" && parseFloat(cs.outlineWidth) > 0;
  parts.push(drawn ? `outline:${cs.outlineStyle} ${cs.outlineWidth} ${cs.outlineColor} ${cs.outlineOffset}` : "outline:none");
  for (const side of SIDES) {
    const st = cs[`border${side}Style`];
    const w = parseFloat(cs[`border${side}Width`]);
    parts.push(st === "none" || st === "hidden" || !(w > 0)
      ? `b${side}:none`
      : `b${side}:${st} ${w} ${cs[`border${side}Color`]}`);
  }
  for (const k of PLAIN) parts.push(cs[k]);
  return parts.join(" ");
}

function one(node, parts) {
  for (const p of PSEUDO) {
    const cs = getComputedStyle(node, p);
    if (p && cs.content === "none") {
      parts.push("absent");
      continue;
    }
    parts.push(painted(cs));
    if (p) parts.push(cs.content, cs.transform, cs.inset);
  }
}

// THE RING IS OFTEN NOT ON THE CONTROL, and in this tree it is usually not even
// on an ancestor. The kit's slider is a transparent `input[type=range]` laid
// over a drawn track, and `:focus-within` on their shared wrapper colours THE
// TRACK - a sibling. An ancestors-only walk saw the wrapper's own style, which
// does not move, and reported every slider in Settings as ringless. So the walk
// covers each ancestor's SUBTREE, which is where a sibling indicator lives.
//
// Bounded, because an ancestor three levels up can be most of a page: nodes are
// taken in document order until the budget runs out, and the budget is per
// element rather than per run so a long page does not starve its own last
// control.
const NODE_BUDGET = 48;

function snap(el) {
  const parts = [];
  const seen = new Set();
  let node = el;
  for (let up = 0; node && up < 4; up++, node = node.parentElement) {
    if (!seen.has(node)) {
      seen.add(node);
      one(node, parts);
    }
    for (const kid of node.querySelectorAll("*")) {
      if (seen.size >= NODE_BUDGET) break;
      if (seen.has(kid)) continue;
      seen.add(kid);
      one(kid, parts);
    }
  }
  return parts.join("|");
}

function label(el) {
  const id = el.id ? "#" + el.id : "";
  const cls = (el.getAttribute("class") || "").trim().split(/\s+/).filter(Boolean).slice(0, 2);
  const text = (el.innerText || el.value || el.getAttribute("aria-label") || "").trim().slice(0, 40);
  return `${el.tagName.toLowerCase()}${id}${cls.length ? "." + cls.join(".") : ""}` +
         (text ? `: "${text}"` : "");
}

// A TRANSITIONED RING IS INVISIBLE TO A SYNCHRONOUS READ, and this probe is a
// synchronous read. `getComputedStyle` right after `focus()` returns the value
// the transition is animating FROM, so a ring that fades in over 150ms - which
// is every ring in this kit, since the field's border-color is transitioned -
// measures as no change at all. The first run reported the Settings search field
// for exactly that reason and the CSS was correct all along.
//
// Turning transitions off document-wide before either snapshot is the fix that
// keeps the probe synchronous: both reads then see the settled value. It is
// symmetric, so it cannot manufacture a difference of its own.
const still = document.createElement("style");
still.textContent = "*, *::before, *::after { transition: none !important; animation: none !important }";
document.head.appendChild(still);

const SEL = "a[href], area[href], button, input, select, textarea, summary, [tabindex], [contenteditable='true']";
const out = [];
let examined = 0;
let unmeasured = 0;
const restore = document.activeElement;

for (const el of document.querySelectorAll(SEL)) {
  // Not focusable, not this probe's business: a disabled control, a negative
  // tabindex (reachable by script, never by Tab), an inert subtree.
  if (el.disabled) continue;
  if (el.tabIndex < 0) continue;
  if (el.closest("[inert]") || el.closest("[aria-hidden='true']")) continue;
  const cs = getComputedStyle(el);
  if (cs.visibility === "hidden" || cs.display === "none") continue;
  const r = el.getBoundingClientRect();
  if (r.width === 0 || r.height === 0) continue;
  // The screen-reader pattern is a 1x1 box. It takes focus on purpose - a skip
  // link is the whole point of one - and has nothing to draw, so counting it
  // would put one in every result.
  //
  // Recognised by WHAT IT PUTS ON SCREEN rather than by its size, because all
  // three size measures lie about it in a different direction.
  // `getBoundingClientRect` on the pattern comes back near seventeen pixels once
  // a button's default border and padding are in. `clientWidth` sees through
  // that and is ZERO for every inline element, which silently dropped every
  // `<a>` in the tree including the control's own unringed link. And the
  // computed `width` is not the declared 1px either: WebKit floors a button's
  // border box at its own padding and reports 16px.
  //
  // So: it holds words, it renders none of them, and it has no child with a box.
  // `innerText` is the measure that can say that - it omits what is not
  // rendered, which is the whole difference from `textContent`. An icon-only
  // button renders no text either and is NOT this: it has a drawn child.
  const shown = (el.innerText || "").trim();
  const held = (el.textContent || "").trim();
  const drawnChild = Array.from(el.children).some((c) => {
    const b = c.getBoundingClientRect();
    return b.width > 2 && b.height > 2;
  });
  if (held && !shown && !drawnChild) continue;

  const before = snap(el);
  try {
    el.focus({ preventScroll: true });
  } catch (e) {
    continue;
  }
  // It refused focus - a different defect, and not one this probe can tell from
  // a browser quirk, so it says nothing rather than guessing.
  if (document.activeElement !== el) continue;
  // SOME CONTROLS NEVER MATCH `:focus-visible` UNDER A SCRIPTED FOCUS, and most
  // rings in this tree are written against it. WebKit's heuristic lets a
  // programmatic focus count as keyboard-visible for text fields, buttons,
  // ranges and colour swatches - measured, all four - but NOT for
  // `input[type=time]`, which it does not treat as a text entry. So the kit's
  // time field came back as ringless when its ring is correct. There is no way
  // to arm the heuristic from script, so these are counted and named rather than
  // guessed at in either direction. (The first reading of this blamed the
  // sweep's `--open` click, which a no-click fixture disproved: the time field
  // fails on its own and the four others pass right beside it.)
  if (!el.matches(":focus-visible")) {
    unmeasured++;
    el.blur();
    continue;
  }
  const after = snap(el);
  examined++;
  if (before === after) out.push(label(el));
  el.blur();
}

// AN EMPTY ANSWER OVER NOTHING IS NOT A CLEAN PAGE. A route that renders no
// focusable control at all comes back `[]` and reads as a pass, which is the
// same vacuity as a gate reporting "0 app(s) checked" and exiting 0 - three of
// those were found in `dev/scripts` in September. A page a keyboard cannot enter
// is a finding in its own right, and a probe that looked at nothing needs to say
// so rather than be counted as coverage.
if (examined === 0 && unmeasured === 0) {
  out.push("no control on this page takes keyboard focus");
}
if (unmeasured > 0) {
  out.push(
    unmeasured + " control(s) not measured: a scripted focus does not match " +
    ":focus-visible on them, which is where their ring would be"
  );
}

if (restore && restore.focus) restore.focus({ preventScroll: true });
still.remove();
return JSON.stringify(out);
