// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The day grid's geometry and the date arithmetic under it. `layoutDay` decides
// whether two meetings at the same hour sit side by side or on top of each
// other, and a screenshot of a week cannot tell a correct column split from a
// lucky one. The date helpers carry a scar worth keeping: `new Date("2026-08-21")`
// is UTC midnight and shows the day BEFORE west of Greenwich, so everything here
// is built from parts.
import { describe, it, expect, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => ({}) }));

import {
  ymd,
  parseYmd,
  addDays,
  startOfWeek,
  minutesOf,
  layoutDay,
} from "./calendar.js";
import type { AgendaEvent } from "./calendar.js";

function ev(uid: string, time: string | null, end: string | null = null): AgendaEvent {
  return {
    uid,
    summary: uid,
    date: "2026-08-21",
    time,
    end_time: end,
    location: "",
    kind: "floating",
    tzid: null,
  } as unknown as AgendaEvent;
}

describe("date helpers", () => {
  it("round-trips a local date through its parts", () => {
    expect(ymd(parseYmd("2026-08-21"))).toBe("2026-08-21");
    // The scar: a date built from parts is local midnight, so the day survives
    // west of Greenwich.
    expect(parseYmd("2026-08-21").getDate()).toBe(21);
  });

  it("adds and subtracts days across a month boundary", () => {
    expect(addDays("2026-08-31", 1)).toBe("2026-09-01");
    expect(addDays("2026-09-01", -1)).toBe("2026-08-31");
    expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
  });

  it("crosses a leap day", () => {
    expect(addDays("2028-02-28", 1)).toBe("2028-02-29");
    expect(addDays("2028-02-29", 1)).toBe("2028-03-01");
  });

  it("finds the Monday of a week, including from a Sunday", () => {
    // 2026-08-21 is a Friday.
    expect(startOfWeek("2026-08-21")).toBe("2026-08-17");
    // A Sunday belongs to the week that STARTED, not the one about to.
    expect(startOfWeek("2026-08-23")).toBe("2026-08-17");
    expect(startOfWeek("2026-08-17")).toBe("2026-08-17");
  });

  it("reads minutes since midnight", () => {
    expect(minutesOf("00:00")).toBe(0);
    expect(minutesOf("09:30")).toBe(570);
    expect(minutesOf("23:59")).toBe(1439);
  });
});

describe("layoutDay", () => {
  it("ignores all-day events, which are not in the time grid", () => {
    expect(layoutDay([ev("allday", null)])).toEqual([]);
  });

  it("gives a lone event the whole width", () => {
    const [b] = layoutDay([ev("one", "09:00", "10:00")]);
    expect(b).toMatchObject({ startMin: 540, endMin: 600, col: 0, cols: 1 });
  });

  it("splits the column between two that overlap", () => {
    const out = layoutDay([ev("a", "09:00", "10:00"), ev("b", "09:30", "10:30")]);
    expect(out.map((b) => b.col)).toEqual([0, 1]);
    // BOTH get the cluster's width, or one would be drawn full-width under the
    // other and the overlap would be invisible.
    expect(out.map((b) => b.cols)).toEqual([2, 2]);
  });

  it("reuses a column once its event has ended", () => {
    const out = layoutDay([
      ev("a", "09:00", "10:00"),
      ev("b", "09:30", "10:30"),
      ev("c", "10:15", "11:00"),
    ]);
    // c starts after a ends, so it takes a's column back - and all three are one
    // cluster, because b bridges them.
    expect(out.map((b) => b.col)).toEqual([0, 1, 0]);
    expect(out.every((b) => b.cols === 2)).toBe(true);
  });

  it("keeps separate clusters separate", () => {
    const out = layoutDay([
      ev("morning", "09:00", "10:00"),
      ev("afternoon", "14:00", "15:00"),
    ]);
    expect(out.every((b) => b.cols === 1)).toBe(true);
  });

  it("gives an event with no end a half hour, and a too-short one a quarter", () => {
    const [open] = layoutDay([ev("open", "09:00")]);
    expect(open.endMin - open.startMin).toBe(30);
    // An end at or before the start would draw a zero-height block nobody can
    // press.
    const [tiny] = layoutDay([ev("tiny", "09:00", "09:00")]);
    expect(tiny.endMin - tiny.startMin).toBe(15);
  });

  it("orders by start, then by the shorter one first", () => {
    const out = layoutDay([
      ev("late", "11:00", "12:00"),
      ev("early-long", "09:00", "12:00"),
      ev("early-short", "09:00", "09:30"),
    ]);
    expect(out.map((b) => b.event.uid)).toEqual(["early-short", "early-long", "late"]);
  });
});
