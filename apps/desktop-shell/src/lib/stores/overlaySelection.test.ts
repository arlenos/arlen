// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The workspace overlay's multi-selection, whose whole contract is about SET
// IDENTITY rather than contents: a Svelte writable compares by reference, so an
// update that mutates in place never renders, and one that copies when nothing
// changed re-renders every card on every windows tick. Both mistakes are
// invisible in a screenshot and neither has a probe. Pinned here.
import { describe, it, expect, beforeEach } from "vitest";
import { get } from "svelte/store";
import { windows, type WindowInfo } from "./windows.js";
import {
  selectedWindowIds,
  validSelection,
  toggleSelection,
  addToSelection,
  selectOnly,
  clearSelection,
  isSelected,
  selectionSnapshot,
  pruneSelection,
} from "./overlaySelection.js";

function win(id: string): WindowInfo {
  return { id, app_id: "dev.arlen.demo", title: id, active: false, workspace_ids: ["ws-1"] };
}

describe("overlay selection", () => {
  beforeEach(() => {
    clearSelection();
    windows.set([]);
  });

  it("toggles an id in and out", () => {
    toggleSelection("a");
    expect(isSelected("a")).toBe(true);
    toggleSelection("a");
    expect(isSelected("a")).toBe(false);
  });

  it("hands out a new set on every real change, so the overlay re-renders", () => {
    const before = get(selectedWindowIds);
    addToSelection("a");
    const after = get(selectedWindowIds);
    expect(after).not.toBe(before);
    expect([...after]).toEqual(["a"]);
  });

  it("keeps the same set when nothing changed, so it does not", () => {
    addToSelection("a");
    const before = get(selectedWindowIds);
    addToSelection("a");
    expect(get(selectedWindowIds)).toBe(before);

    clearSelection();
    const empty = get(selectedWindowIds);
    clearSelection();
    expect(get(selectedWindowIds)).toBe(empty);
  });

  it("selectOnly replaces the whole selection", () => {
    addToSelection("a");
    addToSelection("b");
    selectOnly("c");
    expect(selectionSnapshot()).toEqual(["c"]);
  });

  it("snapshots in insertion order and does not track later changes", () => {
    addToSelection("b");
    addToSelection("a");
    const snap = selectionSnapshot();
    addToSelection("z");
    expect(snap).toEqual(["b", "a"]);
  });

  it("prunes ids whose window is gone, and only then", () => {
    addToSelection("a");
    addToSelection("b");
    const before = get(selectedWindowIds);

    pruneSelection(new Set(["a", "b"]));
    expect(get(selectedWindowIds)).toBe(before);

    pruneSelection(new Set(["a"]));
    expect(selectionSnapshot()).toEqual(["a"]);
  });

  it("drops a window that closed between selecting it and acting on it", () => {
    // The reason `validSelection` exists: a card can be selected and its window
    // gone by the time the context menu fires, and an action against a dead id
    // is worse than an action against fewer windows.
    windows.set([win("a"), win("b")]);
    addToSelection("a");
    addToSelection("b");
    expect([...get(validSelection)].sort()).toEqual(["a", "b"]);

    windows.set([win("a")]);
    expect([...get(validSelection)]).toEqual(["a"]);
    // The raw selection is untouched: pruning is the indicator's job on its own
    // tick, and this view never mutates what it filters.
    expect(selectionSnapshot().sort()).toEqual(["a", "b"]);
  });
});
