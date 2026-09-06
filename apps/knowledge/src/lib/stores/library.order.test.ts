import { describe, expect, it } from "vitest";
import { sortSections, type LibrarySection } from "./library";

const section = (source: string, cls: LibrarySection["class"]): LibrarySection => ({
  source,
  type: `${source}.Thing`,
  label: source,
  class: cls,
  entries: [],
});

describe("library section order", () => {
  it("orders by display class, then by source", () => {
    const sorted = sortSections([
      section("z.media", "media"),
      section("b.notes", "notes"),
      section("a.docs", "documents"),
      section("a.notes", "notes"),
    ]);
    expect(sorted.map((s) => s.source)).toEqual([
      "a.notes",
      "b.notes",
      "a.docs",
      "z.media",
    ]);
  });

  it("gives a class this build has never seen a place at the end", () => {
    // The set is closed, so this cannot arrive from a schema the daemon parsed -
    // it would have been refused there. The point is that the ORDER never drops a
    // section: an unknown class sorts last rather than disappearing, so a build
    // that is behind still shows everything it was sent.
    const sorted = sortSections([
      { ...section("x.future", "notes"), class: "sculpture" as LibrarySection["class"] },
      section("a.notes", "notes"),
    ]);
    expect(sorted.map((s) => s.source)).toEqual(["a.notes", "x.future"]);
  });

  it("does not reorder the caller's array", () => {
    const given = [section("b.notes", "media"), section("a.notes", "notes")];
    sortSections(given);
    expect(given.map((s) => s.source)).toEqual(["b.notes", "a.notes"]);
  });
});
