// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The duplicate finder's review state, which is the one surface in this app
// whose bug is somebody's only copy of a file. The safety floor is a piece of
// LOGIC - "the guard refuses to mark the last one" - and nothing was checking
// it, in a tree where the render probes can say the row is drawn and never that
// pressing it twice leaves a keeper.
import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import {
  duplicateGroups,
  trashMarks,
  markedPaths,
  reclaimable,
  keptCount,
  keepNewest,
  toggleTrash,
  groupReclaimable,
  closeDuplicates,
  type DupGroup,
} from "./duplicates.js";

function group(hash: string, files: [string, number, number | null][]): DupGroup {
  return {
    hash,
    files: files.map(([path, size, modified_unix]) => ({
      path,
      name: path.split("/").pop() ?? path,
      size,
      modified_unix,
    })),
  };
}

const THREE = group("h1", [
  ["/home/u/a.jpg", 100, 10],
  ["/home/u/b.jpg", 100, 30],
  ["/home/u/c.jpg", 100, 20],
]);

describe("duplicate review", () => {
  beforeEach(() => {
    duplicateGroups.set(null);
    trashMarks.set(new Set());
  });

  it("counts what is kept, not what is marked", () => {
    expect(keptCount(THREE, new Set())).toBe(3);
    expect(keptCount(THREE, new Set(["/home/u/a.jpg"]))).toBe(2);
  });

  it("refuses to mark the last kept copy in a group", () => {
    // The floor: however many times a person presses, one copy survives.
    toggleTrash(THREE, "/home/u/a.jpg");
    toggleTrash(THREE, "/home/u/c.jpg");
    expect(keptCount(THREE, get(trashMarks))).toBe(1);

    toggleTrash(THREE, "/home/u/b.jpg");
    expect(keptCount(THREE, get(trashMarks))).toBe(1);
    expect(get(trashMarks).has("/home/u/b.jpg")).toBe(false);
  });

  it("always lets the keeper be un-marked", () => {
    toggleTrash(THREE, "/home/u/a.jpg");
    expect(get(trashMarks).has("/home/u/a.jpg")).toBe(true);
    toggleTrash(THREE, "/home/u/a.jpg");
    expect(get(trashMarks).has("/home/u/a.jpg")).toBe(false);
  });

  it("keeps the newest copy and marks the rest", () => {
    duplicateGroups.set([THREE]);
    keepNewest();
    // b is newest (30), so a and c are marked.
    expect([...get(trashMarks)].sort()).toEqual(["/home/u/a.jpg", "/home/u/c.jpg"]);
    expect(keptCount(THREE, get(trashMarks))).toBe(1);
  });

  it("treats a copy with no timestamp as the oldest rather than the keeper", () => {
    // A file the backend could not stamp must not become the one thing kept, or
    // the preset silently trashes every dated copy in favour of an unknown.
    const g = group("h2", [
      ["/x/dated.txt", 5, 100],
      ["/x/undated.txt", 5, null],
    ]);
    duplicateGroups.set([g]);
    keepNewest();
    expect([...get(trashMarks)]).toEqual(["/x/undated.txt"]);
  });

  it("adds up only what is marked, per group and overall", () => {
    const other = group("h3", [
      ["/y/one.bin", 7, 1],
      ["/y/two.bin", 7, 2],
    ]);
    duplicateGroups.set([THREE, other]);
    trashMarks.set(new Set(["/home/u/a.jpg", "/y/one.bin"]));

    expect(groupReclaimable(THREE, get(trashMarks))).toBe(100);
    expect(groupReclaimable(other, get(trashMarks))).toBe(7);
    expect(get(reclaimable)).toBe(107);
    expect(get(markedPaths).sort()).toEqual(["/home/u/a.jpg", "/y/one.bin"]);
  });

  it("forgets every mark when the view closes", () => {
    duplicateGroups.set([THREE]);
    trashMarks.set(new Set(["/home/u/a.jpg"]));
    closeDuplicates();
    expect(get(trashMarks).size).toBe(0);
    expect(get(duplicateGroups)).toBe(null);
    expect(get(reclaimable)).toBe(0);
  });
});
