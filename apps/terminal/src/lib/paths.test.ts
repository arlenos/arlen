// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Four ways of writing the same path short enough for the place it appears: a
// sidebar row, a prompt, a listing. All four are presentation, and every one of
// them can drop the part a person needed - the leaf. The render probes can say a
// row was not clipped; only this can say the shortening kept the right end.
import { describe, it, expect } from "vitest";
import { tildify, shortPath, collapsePath, displayPath } from "./paths.js";

describe("tildify", () => {
  it("writes a home path with a tilde", () => {
    expect(tildify("/home/tim")).toBe("~");
    expect(tildify("/home/tim/Downloads")).toBe("~/Downloads");
  });

  it("leaves a path outside home alone", () => {
    expect(tildify("/usr/share/arlen")).toBe("/usr/share/arlen");
    expect(tildify("/homely/tim")).toBe("/homely/tim");
    // Another person's home is not this person's tilde.
    expect(tildify("/home")).toBe("/home");
  });
});

describe("shortPath", () => {
  it("keeps a short path whole", () => {
    expect(shortPath("/home/tim")).toBe("~");
    expect(shortPath("/home/tim/Downloads")).toBe("~/Downloads");
  });

  it("keeps the last two segments of a deep one", () => {
    expect(shortPath("/home/tim/Repositories/arlen")).toBe("Repositories/arlen");
    expect(shortPath("/usr/share/arlen/apps")).toBe("arlen/apps");
  });

  it("calls the root the root", () => {
    expect(shortPath("/")).toBe("/");
  });
});

describe("collapsePath", () => {
  it("abbreviates the trail and keeps the current folder whole", () => {
    expect(collapsePath("/home/tim/Repositories/arlen/apps/terminal")).toEqual({
      prefix: "~/R/a/apps/",
      anchor: "terminal",
    });
  });

  it("leaves a shallow path alone", () => {
    expect(collapsePath("/home/tim/Repositories/arlen")).toEqual({
      prefix: "~/Repositories/",
      anchor: "arlen",
    });
  });

  it("has an anchor even at home and at the root", () => {
    expect(collapsePath("/home/tim")).toEqual({ prefix: "", anchor: "~" });
    expect(collapsePath("/")).toEqual({ prefix: "", anchor: "/" });
  });
});

describe("displayPath", () => {
  it("keeps a path that fits", () => {
    expect(displayPath("/home/tim/Repositories/arlen")).toBe("~/Repositories/arlen");
  });

  it("drops whole leading segments and always keeps the leaf", () => {
    const got = displayPath("/home/tim/Repositories/arlen/apps/terminal/src", 20);
    expect(got.endsWith("src")).toBe(true);
    expect(got.startsWith("…/")).toBe(true);
    expect(got.length).toBeLessThanOrEqual(20);
  });

  it("keeps the honest full form when the ellipsis would not shorten it", () => {
    // A one-segment path cannot lose a leading segment, so "…/x" is longer than
    // "x" and says less. The full name wins.
    expect(displayPath("/verylongsinglesegmentname", 5)).toBe("/verylongsinglesegmentname");
  });

  it("never returns something longer than what it was given", () => {
    for (const p of ["/", "/home/tim", "/home/tim/a/b/c/d/e/f", "/usr/lib/arlen/apps/notes/bin"]) {
      expect(displayPath(p, 12).length).toBeLessThanOrEqual(tildify(p).length);
    }
  });
});
