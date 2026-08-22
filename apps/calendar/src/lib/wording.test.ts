// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The three rules this window applies to a row. Two are about somebody's clock,
// which is exactly the kind of thing that is right for eleven months and wrong
// on the day the month turns - and none of them could be exercised at all while
// they lived inside the component.

import { describe, expect, it } from "vitest";
import { dayLabel, isToday, repeatLabel } from "./wording";

const echo = (key: string, values?: Record<string, unknown>) =>
  values ? `${key}:${JSON.stringify(values)}` : key;

describe("isToday", () => {
  it("marks the reader's own day", () => {
    // Local noon, so no timezone can push the date either way.
    expect(isToday("2026-08-21", new Date(2026, 7, 21, 12, 0))).toBe(true);
  });

  it("does not mark yesterday or tomorrow", () => {
    const now = new Date(2026, 7, 21, 12, 0);
    expect(isToday("2026-08-20", now)).toBe(false);
    expect(isToday("2026-08-22", now)).toBe(false);
  });

  it("pads a single-digit month and day, so March 3rd is not 2026-3-3", () => {
    expect(isToday("2026-03-03", new Date(2026, 2, 3, 9, 0))).toBe(true);
  });

  it("uses the reader's local date, not UTC", () => {
    // A minute before midnight local is still today for the person reading it,
    // whatever UTC thinks. Constructed in local terms on purpose.
    const lateOn21 = new Date(2026, 7, 21, 23, 59);
    expect(isToday("2026-08-21", lateOn21)).toBe(true);
    expect(isToday("2026-08-22", lateOn21)).toBe(false);
  });
});

describe("dayLabel", () => {
  it("names the weekday in the reader's language", () => {
    expect(dayLabel("2026-08-21", "en-GB")).toContain("Friday");
    expect(dayLabel("2026-08-21", "de-DE")).toContain("Freitag");
  });

  it("shows the day the string names, not the day before", () => {
    // `new Date("2026-08-21")` is UTC midnight and reads as the 20th anywhere
    // west of Greenwich; this builds the date from its parts instead.
    expect(dayLabel("2026-08-21", "en-GB")).toContain("21");
  });
});

describe("repeatLabel", () => {
  it("says how often, not merely that it repeats", () => {
    expect(repeatLabel({ every: "weekly", every_n: 1, on_days: [] }, echo)).toBe(
      'cal.everyWeekly:{"n":1}',
    );
  });

  it("names the days a weekly rule fires on", () => {
    const out = repeatLabel({ every: "weekly", every_n: 1, on_days: ["mon", "wed"] }, echo);
    expect(out).toContain("cal.onDays");
    expect(out).toContain("cal.dayMon");
    expect(out).toContain("cal.dayWed");
  });

  it("falls back to the bare word for a rule the calendar refused", () => {
    // No frequency means nothing was worked out. Vague beats wrong about
    // somebody's week.
    expect(repeatLabel({ every: null, every_n: 1, on_days: [] }, echo)).toBe("cal.repeats");
  });

  it("shows a weekday key it does not know as written", () => {
    const out = repeatLabel({ every: "weekly", every_n: 2, on_days: ["xyz"] }, echo);
    expect(out).toContain("xyz");
  });
});
