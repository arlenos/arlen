/// The conformance suite: the seven tests of
/// `apps/files/core/src/selection.rs` ported verbatim, plus the one
/// documented divergence (`remap`). If a case here disagrees with the
/// Rust tests, the Rust model wins.
import { describe, expect, it } from "vitest";
import { Selection } from "./selection";

describe("selection conformance with core/src/selection.rs", () => {
  it("click selects only one and sets anchor and cursor", () => {
    const s = new Selection(5);
    s.click(2);
    expect(s.indices()).toEqual([2]);
    expect(s.cursor()).toBe(2);
    s.click(4);
    expect(s.indices()).toEqual([4]);
    s.click(9);
    expect(s.indices()).toEqual([4]);
  });

  it("ctrl toggle adds and removes keeping the rest", () => {
    const s = new Selection(5);
    s.click(1);
    s.toggle(3);
    s.toggle(4);
    expect(s.indices()).toEqual([1, 3, 4]);
    s.toggle(3);
    expect(s.indices()).toEqual([1, 4]);
  });

  it("shift range selects from the anchor inclusive both directions", () => {
    const s = new Selection(8);
    s.click(2);
    s.rangeTo(5);
    expect(s.indices()).toEqual([2, 3, 4, 5]);
    s.rangeTo(0);
    expect(s.indices()).toEqual([0, 1, 2]);
  });

  it("arrow moves cursor and single selects clamped", () => {
    const s = new Selection(3);
    s.moveCursor(1, false);
    expect([s.cursor(), s.indices()]).toEqual([1, [1]]);
    s.moveCursor(1, false);
    s.moveCursor(1, false);
    expect([s.cursor(), s.indices()]).toEqual([2, [2]]);
    s.moveCursor(-Infinity, false);
    expect(s.cursor()).toBe(0);
    s.moveCursor(Infinity, false);
    expect(s.cursor()).toBe(2);
  });

  it("shift arrow extends the range from the anchor", () => {
    const s = new Selection(6);
    s.click(1);
    s.moveCursor(1, true);
    s.moveCursor(1, true);
    expect(s.indices()).toEqual([1, 2, 3]);
    expect(s.cursor()).toBe(3);
    s.moveCursor(-3, true);
    expect(s.indices()).toEqual([0, 1]);
  });

  it("select all and clear", () => {
    const s = new Selection(4);
    s.selectAll();
    expect(s.indices()).toEqual([0, 1, 2, 3]);
    expect(s.cursor()).toBe(3);
    s.clear();
    expect(s.indices()).toEqual([]);
    expect(s.cursor()).toBe(3);
  });

  it("rebase clears and an empty listing is inert", () => {
    const s = new Selection(4);
    s.click(2);
    s.rebase(10);
    expect(s.indices()).toEqual([]);
    expect(s.cursor()).toBe(null);
    const empty = new Selection(0);
    empty.moveCursor(1, false);
    empty.click(0);
    empty.selectAll();
    expect(empty.indices()).toEqual([]);
    expect(empty.cursor()).toBe(null);
  });
});

describe("the documented divergence: remap on re-sort", () => {
  it("carries the selection by name across a re-list of the same dir", () => {
    const s = new Selection(3);
    const before = ["a.md", "b.md", "c.md"];
    s.click(0);
    s.toggle(2);
    s.remap(before, ["c.md", "b.md", "a.md"]);
    expect(s.indices()).toEqual([0, 2]);
    expect(s.cursor()).toBe(0);
  });

  it("drops names that vanished (hidden toggled off)", () => {
    const s = new Selection(3);
    s.click(1);
    s.remap([".hidden", "a.md", "b.md"], ["a.md", "b.md"]);
    expect(s.indices()).toEqual([0]);
  });
});
