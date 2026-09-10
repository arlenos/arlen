// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// Which words the content header and the empty state use, for a location that
// may be a place or a refinement OF a place. A saved search is `search:<q>` and
// still belongs under Searches; getting that wrong shows a person the timeline's
// heading over their own saved query, which reads as a navigation that went
// somewhere else.
import { describe, it, expect } from "vitest";
import { EXPLORE_PLACES, labelKeyFor, emptyKeyFor } from "./locations.js";

describe("knowledge places", () => {
  it("names each place in sidebar order", () => {
    expect(EXPLORE_PLACES.map((p) => p.id)).toEqual([
      "timeline",
      "projects",
      "searches",
      "library",
    ]);
  });

  it("gives every place both of its keys", () => {
    for (const p of EXPLORE_PLACES) {
      expect(p.labelKey.startsWith("k.place.")).toBe(true);
      expect(p.emptyKey.startsWith("k.empty.")).toBe(true);
    }
  });

  it("resolves a place to its own words", () => {
    expect(labelKeyFor("projects")).toBe("k.place.projects");
    expect(emptyKeyFor("library")).toBe("k.empty.library");
  });

  it("resolves a refinement back to the place it refines", () => {
    expect(labelKeyFor("search:invoices 2025")).toBe("k.place.searches");
    expect(emptyKeyFor("search:invoices 2025")).toBe("k.empty.searches");
    expect(labelKeyFor("project:abc-123")).toBe("k.place.projects");
    expect(emptyKeyFor("project:abc-123")).toBe("k.empty.projects");
  });

  it("keeps a query that contains a colon whole", () => {
    // The scheme is the part before the FIRST colon, and a saved search is free
    // text: "search:http://example.com" is one query, not a place called http.
    expect(labelKeyFor("search:http://example.com")).toBe("k.place.searches");
  });

  it("falls back to the app's own title for a location it does not know", () => {
    expect(labelKeyFor("nowhere")).toBe("k.title");
    expect(emptyKeyFor("nowhere")).toBe("k.empty.timeline");
  });
});
