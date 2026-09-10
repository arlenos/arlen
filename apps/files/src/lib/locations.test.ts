// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// What a virtual location calls itself, which columns it shows and which
// sentence it says when it is empty or refuses. Four functions branching on the
// same five prefixes, and the one that matters most is the failure title: "Can't
// open this folder" is true of a directory and false of Recent, which is a list
// the knowledge graph answers with and has no folder anywhere.
import { describe, it, expect } from "vitest";
import { locationLabel, columnsFor, errorTitleFor, emptyLabelFor } from "./locations.js";
import type { PlaceGroup } from "@arlen/ui-kit/components/browser";

//: Reveals the key and its parameters, because the key IS the decision - the
//: German catalogue writes these differently and this module deliberately does
//: not know how.
const t = ((key: string, params?: Record<string, unknown>) =>
  params ? `${key}(${Object.entries(params).map(([k, v]) => `${k}=${v}`).join(",")})` : key) as never;

const groups: PlaceGroup[] = [
  {
    label: "Projects",
    places: [
      { path: "project:p1", label: "Hausbau", icon: "folder" },
      { path: "facet:f1", label: "Große Bilder", icon: "folder" },
    ],
  },
] as unknown as PlaceGroup[];

describe("locationLabel", () => {
  it("names the two fixed places", () => {
    expect(locationLabel(t, "recent")).toBe("f.loc.recent");
    expect(locationLabel(t, "trash")).toBe("f.loc.trash");
  });

  it("carries the query into the search label, and drops it when empty", () => {
    expect(locationLabel(t, "search:rechnung")).toBe("f.loc.searchFor(query=rechnung)");
    expect(locationLabel(t, "search:   ")).toBe("f.loc.search");
  });

  it("resolves a project to its place name, or shows the bare id", () => {
    expect(locationLabel(t, "project:p1", groups)).toBe("Hausbau");
    expect(locationLabel(t, "project:unknown", groups)).toBe("unknown");
  });

  it("resolves a saved filter to its name, and an ad hoc one to Filtered", () => {
    expect(locationLabel(t, "facet:f1", groups)).toBe("Große Bilder");
    expect(locationLabel(t, "facet:anything-else", groups)).toBe("f.loc.filtered");
  });

  it("leaves a real path alone", () => {
    expect(locationLabel(t, "/home/u/Bilder")).toBe("/home/u/Bilder");
  });
});

describe("columnsFor", () => {
  it("swaps Size for the item's home folder wherever the items are scattered", () => {
    for (const path of ["trash", "recent", "project:p1", "search:x", "facet:f1"]) {
      expect(columnsFor(path).middle).toBe("location");
    }
    expect(columnsFor("/home/u").middle).toBe("size");
  });

  it("relabels the time column per location", () => {
    expect(columnsFor("trash").timeLabel).toBe("f.col.deleted");
    expect(columnsFor("recent").timeLabel).toBe("f.col.lastAccessed");
    expect(columnsFor("/home/u").timeLabel).toBe("f.col.modified");
  });

  it("says where a trashed item came from, not merely where it is", () => {
    expect(columnsFor("trash").middleLabel).toBe("f.col.originalLocation");
    expect(columnsFor("recent").middleLabel).toBe("f.col.location");
  });
});

describe("errorTitleFor", () => {
  it("does not call a virtual location a folder", () => {
    for (const path of ["recent", "trash", "project:p1", "search:x", "facet:f1"]) {
      expect(errorTitleFor(path)).toBe("f.fb.errorTitleLocation");
    }
  });

  it("calls a directory a folder", () => {
    expect(errorTitleFor("/home/u/Dokumente")).toBe("f.fb.errorTitle");
  });
});

describe("emptyLabelFor", () => {
  it("gives each location its own empty sentence", () => {
    expect(emptyLabelFor("trash")).toBe("f.empty.trash");
    expect(emptyLabelFor("recent")).toBe("f.empty.recent");
    expect(emptyLabelFor("project:p1")).toBe("f.empty.project");
    expect(emptyLabelFor("search:x")).toBe("f.empty.search");
    expect(emptyLabelFor("facet:f1")).toBe("f.empty.facet");
    expect(emptyLabelFor("/home/u")).toBe("f.empty.folder");
  });
});
