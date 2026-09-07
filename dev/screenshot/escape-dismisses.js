// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Does Escape close what is open?
//
// Point it at a surface with `--open` and it reports what overlay was showing
// before the key and what is showing after:
//
//   dev/screenshot/headless.sh --url http://localhost:5310/?locale=de \
//     --out /tmp/x.png --open '[data-applet-id=network]' \
//     --probe-file dev/screenshot/escape-dismisses.js
//
// WHY A PROBE AND NOT A READ. A handler that looks right can be unreachable, and
// on 8 September one was: the window-rule dialog in Settings mounted its Escape
// handler on its own backdrop, which only receives a key press if focus is
// already inside it. The dialog does not take focus when it opens, so the press
// landed on `body` and the handler never ran - and every reader of that file,
// including the one who wrote it, saw a dialog that handled Escape. Driven, it
// answered `focus=BODY, open before=true, open after=true`.
//
// It needs `render-wide.py`'s awaited probes: Svelte flushes the DOM in a
// microtask, so a probe that looks immediately sees the page before the key had
// any effect, and one that spins to wait blocks the very microtask it waits for.
// Both were tried, and both report that nothing happened.
//
// THREE TARGETS, because a dismissal can be wired to any of them: the window (a
// `<svelte:window onkeydown>`), the document (a listener added on mount), and
// the focused element (a handler on the overlay itself, which is the form that
// works only when something inside has focus).
const SEL = [
  ".pop-panel",
  ".qs-panel",
  '[data-slot*="popover-content"]',
  "[data-popover-content]",
  '[role="dialog"]',
  '[role="menu"]',
  '[role="listbox"]',
  ".menu",
  ".palette",
].join(", ");

// `offsetParent` is null for anything `position: fixed`, which is every overlay
// here - it reported all of them hidden on the first run.
const vis = (e) => e.getClientRects().length > 0 && getComputedStyle(e).visibility !== "hidden";

const shown = () =>
  [...document.querySelectorAll(SEL)]
    .filter(vis)
    .map((e) => (e.getAttribute("aria-label") || String(e.className).split(/\s+/)[0] || e.tagName).slice(0, 24));

const before = shown();
const focused = document.activeElement ? document.activeElement.tagName : "none";
const key = () => new KeyboardEvent("keydown", { key: "Escape", bubbles: true });
window.dispatchEvent(key());
document.dispatchEvent(key());
(document.activeElement || document.body).dispatchEvent(key());

return new Promise((resolve) =>
  setTimeout(
    () => resolve([`focus=${focused}`, `before=[${before.join("|")}]`, `after=[${shown().join("|")}]`]),
    450,
  ),
);
