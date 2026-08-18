import { describe, expect, it } from "vitest";
import { pinnedOrder, rowMatches } from "./freeze";

const row = (id: number) => ({ id });

describe("pinnedOrder", () => {
  it("holds the order it was given, whatever the fresh sort says", () => {
    // The live sort has reordered everything; the pin says otherwise.
    const fresh = [row(3), row(1), row(2)];
    expect(pinnedOrder(fresh, [1, 2, 3]).map((r) => r.id)).toEqual([1, 2, 3]);
  });

  it("keeps the same objects, so the values underneath stay live", () => {
    // The point of freezing the ORDER and not the data: the row at position 0
    // must be the FRESH row 1, carrying this poll's numbers, not a stale copy.
    const fresh = [{ id: 1, cpu: 9 }];
    const [held] = pinnedOrder(fresh, [1]);
    expect(held).toBe(fresh[0]);
    expect(held.cpu).toBe(9);
  });

  it("appends processes that started while frozen, rather than hiding them", () => {
    const fresh = [row(9), row(1), row(2)];
    expect(pinnedOrder(fresh, [1, 2]).map((r) => r.id)).toEqual([1, 2, 9]);
  });

  it("appends several newcomers in the live order, so the busiest is first", () => {
    // `fresh` is sorted CPU-desc, so 8 outranks 9 among the new arrivals.
    const fresh = [row(8), row(9), row(1)];
    expect(pinnedOrder(fresh, [1]).map((r) => r.id)).toEqual([1, 8, 9]);
  });

  it("drops a process that exited instead of leaving a hole for it", () => {
    const fresh = [row(1), row(3)];
    expect(pinnedOrder(fresh, [1, 2, 3]).map((r) => r.id)).toEqual([1, 3]);
  });

  it("is the live order when nothing was pinned", () => {
    const fresh = [row(3), row(1)];
    expect(pinnedOrder(fresh, []).map((r) => r.id)).toEqual([3, 1]);
  });

  it("returns every row exactly once, even when the pin has stale ids", () => {
    // A pin captured two polls ago can name ids that are gone AND miss ids that
    // arrived. Neither may duplicate or lose a row.
    const fresh = [row(5), row(1), row(7)];
    const out = pinnedOrder(fresh, [1, 99, 5]);
    expect(out.map((r) => r.id).sort()).toEqual([1, 5, 7]);
    expect(out).toHaveLength(3);
  });
});

describe("rowMatches", () => {
  const firefox = {
    name: "firefox",
    children: [{ name: "Arlen OS - Wikipedia" }, { name: "Design docs" }],
  };

  it("keeps every row when the box is empty or blank", () => {
    expect(rowMatches(firefox, "")).toBe(true);
    expect(rowMatches(firefox, "   ")).toBe(true);
  });

  it("matches a row on its own name, case-insensitively", () => {
    expect(rowMatches(firefox, "FIRE")).toBe(true);
  });

  it("surfaces the parent when the query matches a CHILD", () => {
    // The clause a drive cannot check on a machine whose children share their
    // parent's name: here the child is a tab title and the parent is not.
    expect(rowMatches(firefox, "wikipedia")).toBe(true);
    expect(rowMatches(firefox, "design")).toBe(true);
  });

  it("refuses a row that matches neither itself nor any child", () => {
    expect(rowMatches(firefox, "zzzznomatch")).toBe(false);
  });

  it("handles a row with no children at all", () => {
    expect(rowMatches({ name: "systemd" }, "systemd")).toBe(true);
    expect(rowMatches({ name: "systemd" }, "kernel")).toBe(false);
  });
});
