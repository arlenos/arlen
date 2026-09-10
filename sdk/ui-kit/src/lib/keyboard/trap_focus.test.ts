// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// `trapFocus` is the kit's answer to the defect class found on 11 September: an
// overlay that says `aria-modal="true"` while the keyboard is still out on the
// page behind it, and an overlay that closes without handing focus back. Five
// surfaces use it now - the modal shell every dialog sits in, the shell's menu
// palette, both terminal palettes and the task manager's row menu - and it had
// no test, so the three halves of the property are pinned here.
import { describe, it, expect, beforeEach } from "vitest";
import { trapFocus } from "./trap_focus.js";

function overlay(): { node: HTMLElement; first: HTMLButtonElement; last: HTMLButtonElement } {
  const node = document.createElement("div");
  const first = document.createElement("button");
  first.textContent = "first";
  const middle = document.createElement("input");
  const last = document.createElement("button");
  last.textContent = "last";
  node.append(first, middle, last);
  document.body.append(node);
  // jsdom reports `offsetParent` as null for everything, and the action filters
  // on it - so the elements are made to look laid out, the way they are in a
  // browser where this runs.
  for (const el of [first, middle, last]) {
    Object.defineProperty(el, "offsetParent", { get: () => node, configurable: true });
  }
  return { node, first, last };
}

describe("trapFocus", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
  });

  it("moves the keyboard into the overlay when it opens", () => {
    const { node, first } = overlay();
    const handle = trapFocus(node, undefined);
    // The action focuses in a microtask, so it never fights a consumer's own
    // autofocus that has not run yet.
    return Promise.resolve().then(() => {
      expect(document.activeElement).toBe(first);
      handle?.destroy?.();
    });
  });

  it("does not fight a focus the overlay already holds", () => {
    const { node, last } = overlay();
    last.focus();
    const handle = trapFocus(node, undefined);
    return Promise.resolve().then(() => {
      expect(document.activeElement).toBe(last);
      handle?.destroy?.();
    });
  });

  it("gives the keyboard back to whatever opened it", () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    expect(document.activeElement).toBe(opener);

    const { node } = overlay();
    const handle = trapFocus(node, undefined);
    return Promise.resolve().then(() => {
      expect(document.activeElement).not.toBe(opener);
      handle?.destroy?.();
      expect(document.activeElement).toBe(opener);
    });
  });

  it("keeps the keyboard where it is when the caller says not to return it", () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    const { node, first } = overlay();
    const handle = trapFocus(node, { returnFocus: false });
    return Promise.resolve().then(() => {
      handle?.destroy?.();
      expect(document.activeElement).toBe(first);
    });
  });

  it("wraps Tab from the last control back to the first", () => {
    const { node, first, last } = overlay();
    const handle = trapFocus(node, undefined);
    last.focus();
    node.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    expect(document.activeElement).toBe(first);
    handle?.destroy?.();
  });

  it("wraps Shift+Tab from the first control back to the last", () => {
    const { node, first, last } = overlay();
    const handle = trapFocus(node, undefined);
    first.focus();
    node.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Tab", shiftKey: true, bubbles: true }),
    );
    expect(document.activeElement).toBe(last);
    handle?.destroy?.();
  });
});
