import { describe, it, expect } from "vitest";
import { groupSessions } from "./session-groups";
import type { Session } from "$lib/stores/conversation";

// A fixed reference instant so the buckets are deterministic: midday, so
// "today" has room on both sides.
const NOW = new Date(2026, 5, 27, 12, 0, 0).getTime();
const DAY = 86_400_000;

function s(id: string, createdAt: number, pinned?: boolean): Session {
  return { id, title: id, createdAt, messages: [], ...(pinned ? { pinned } : {}) };
}

describe("groupSessions", () => {
  it("buckets by creation time and only emits non-empty sections", () => {
    const out = groupSessions(
      [
        s("today", NOW - 1000),
        s("yest", NOW - DAY),
        s("week", NOW - 3 * DAY),
        s("old", NOW - 40 * DAY),
      ],
      NOW,
    );
    expect(out.map((g) => g.label)).toEqual([
      "h.group.today",
      "h.group.yesterday",
      "h.group.previous7",
      "h.group.earlier",
    ]);
  });

  it("floats pinned conversations into a single top section, whole", () => {
    const out = groupSessions(
      [s("a", NOW - 1000), s("old-pin", NOW - 40 * DAY, true), s("b", NOW - 1000)],
      NOW,
    );
    expect(out[0].label).toBe("h.group.pinned");
    expect(out[0].sessions.map((x) => x.id)).toEqual(["old-pin"]);
    // The pinned one does not also appear in "Earlier".
    expect(out.find((g) => g.label === "h.group.earlier")).toBeUndefined();
    expect(out[1].sessions.map((x) => x.id)).toEqual(["a", "b"]);
  });

  it("preserves the incoming order within each section", () => {
    const out = groupSessions([s("first", NOW - 100), s("second", NOW - 200)], NOW);
    expect(out[0].sessions.map((x) => x.id)).toEqual(["first", "second"]);
  });

  it("drops a session created just before midnight into Yesterday", () => {
    const justBeforeMidnight = new Date(2026, 5, 27, 0, 0, 0).getTime() - 1;
    const out = groupSessions([s("late", justBeforeMidnight)], NOW);
    expect(out.map((g) => g.label)).toEqual(["h.group.yesterday"]);
  });

  it("returns nothing for an empty list", () => {
    expect(groupSessions([], NOW)).toEqual([]);
  });
});
