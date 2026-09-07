// A container that takes focus and grows the browser's ring around itself.
//
// The mirror of `no-focus-ring.js`, and the more common defect in this tree. A
// dialog, an overlay or a switcher takes focus so a keyboard lands inside it and
// Escape reaches its handler - which is right, and it was measured wrong in four
// places before anybody looked. WebKit rings whatever is focused, and on a
// CONTAINER that is a 4.8px reddish outline round the whole surface, which reads
// as an error state on a plain "Add window rule" dialog.
//
// All four of this tree's container-focus sites had it on 8 September: three
// Settings dialogs and the workspace switcher. `:focus-visible` does not help -
// a programmatic focus on a `tabindex="-1"` container matches it - so the fix is
// `outline: none` on the container, with the ring left where it belongs, on the
// controls inside.
//
// WHAT IT READS. Only `document.activeElement`, deliberately. A container ring
// exists only when something actually focused one, so synthesising focus on every
// div would report designs nobody ships. That makes `[]` a true clean answer on
// a page with nothing focused, and the control page is what proves the probe can
// still speak.
//
// WHAT IT IS NOT. It does not judge a ring on a real control - that is the other
// probe's question, from the other side. And it says nothing about whether a
// container SHOULD take focus; when one does, this is only about what it then
// looks like.
const el = document.activeElement;
const out = [];

// The things a ring belongs on. A container that carries one of these roles is
// acting as a control and is not this probe's business.
const CONTROL_TAGS = ["INPUT", "BUTTON", "A", "SELECT", "TEXTAREA", "SUMMARY"];
const CONTROL_ROLES = [
  "button", "link", "textbox", "checkbox", "radio", "switch", "slider",
  "menuitem", "option", "tab", "combobox", "searchbox", "spinbutton",
];

function label(e) {
  const cls = (e.className || "").toString().trim().split(/\s+/)[0] || "";
  const id = e.id ? `#${e.id}` : "";
  return `${e.tagName.toLowerCase()}${id}${cls ? "." + cls : ""}`;
}

if (el && el !== document.body && el !== document.documentElement) {
  const role = (el.getAttribute("role") || "").toLowerCase();
  const isControl =
    CONTROL_TAGS.includes(el.tagName) ||
    CONTROL_ROLES.includes(role) ||
    el.isContentEditable;
  if (!isControl) {
    const s = getComputedStyle(el);
    // `outline-style: none` is the fixed state; a zero width with a style set is
    // the same thing drawn by nobody. Both pass.
    const width = parseFloat(s.outlineWidth) || 0;
    if (s.outlineStyle !== "none" && width > 0) {
      out.push(
        `${label(el)} takes focus and draws ${s.outlineStyle} ${s.outlineWidth} ` +
        `${s.outlineColor} round itself; the ring belongs on the controls inside`
      );
    }
  }
}

return JSON.stringify(out);
