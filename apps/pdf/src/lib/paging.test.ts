// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The two places this reader can be off by one: the ends of a document, and what
// Shift does to the space bar. The drive presses a key and reads the page number
// back, which proves the wiring; these are the cases it would be expensive to
// enumerate that way.

import { describe, expect, it } from "vitest";
import { clampPage, pageIntent } from "./paging";

describe("pageIntent", () => {
  it("reads the forward keys as forward", () => {
    for (const key of ["ArrowRight", "ArrowDown", "PageDown"]) {
      expect(pageIntent(key, false, false)).toEqual({ kind: "step", delta: 1 });
    }
  });

  it("reads the back keys as back", () => {
    for (const key of ["ArrowLeft", "ArrowUp", "PageUp"]) {
      expect(pageIntent(key, false, false)).toEqual({ kind: "step", delta: -1 });
    }
  });

  it("sends Space forward and Shift+Space back", () => {
    expect(pageIntent(" ", false, false)).toEqual({ kind: "step", delta: 1 });
    expect(pageIntent(" ", true, false)).toEqual({ kind: "step", delta: -1 });
  });

  it("takes Home and End to the ends", () => {
    expect(pageIntent("Home", false, false)).toEqual({ kind: "first" });
    expect(pageIntent("End", false, false)).toEqual({ kind: "last" });
  });

  it("leaves every key alone while the search box has focus", () => {
    // The one that matters: a reader typing "space" into a search box must not
    // have the page turn under them.
    for (const key of [" ", "ArrowRight", "Home", "End", "PageDown"]) {
      expect(pageIntent(key, false, true)).toBe(null);
    }
  });

  it("is not interested in other keys", () => {
    expect(pageIntent("a", false, false)).toBe(null);
    expect(pageIntent("Enter", false, false)).toBe(null);
  });
});

describe("clampPage", () => {
  it("moves within the document", () => {
    expect(clampPage(3, 1, 10)).toBe(4);
    expect(clampPage(3, -1, 10)).toBe(2);
  });

  it("stops at the last page rather than wrapping to the first", () => {
    // A reader who presses Right on the last page of a report has reached the
    // end of it; page one would read as the document having restarted.
    expect(clampPage(10, 1, 10)).toBe(10);
  });

  it("stops at the first page rather than wrapping to the last", () => {
    expect(clampPage(1, -1, 10)).toBe(1);
  });

  it("holds on a one-page document", () => {
    expect(clampPage(1, 1, 1)).toBe(1);
    expect(clampPage(1, -1, 1)).toBe(1);
  });
});
