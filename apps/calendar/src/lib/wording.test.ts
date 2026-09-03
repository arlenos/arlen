// SPDX-FileCopyrightText: 2026 Tim Kicker
//
// SPDX-License-Identifier: AGPL-3.0-only
//
// The three rules this window applies to a row. Two are about somebody's clock,
// which is exactly the kind of thing that is right for eleven months and wrong
// on the day the month turns - and none of them could be exercised at all while
// they lived inside the component.

import { describe, expect, it } from "vitest";
import { dayLabel, isToday, isoWeek, reminderLabel, repeatLabel } from "./wording";

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

describe("reminderLabel", () => {
  it("says a span in the largest unit that divides it", () => {
    expect(reminderLabel({ trigger: { seconds: -600, related: "start" } }, echo, "en-GB")).toBe(
      'cal.remind.beforeStart:{"span":"cal.span.minutes:{\\"n\\":10}"}',
    );
    expect(reminderLabel({ trigger: { seconds: -3600, related: "start" } }, echo, "en-GB")).toContain(
      'cal.span.hours:{\\"n\\":1}',
    );
    expect(reminderLabel({ trigger: { seconds: -86400, related: "start" } }, echo, "en-GB")).toContain(
      'cal.span.days:{\\"n\\":1}',
    );
    // Ninety minutes is not a whole hour, so it stays minutes.
    expect(reminderLabel({ trigger: { seconds: -5400, related: "start" } }, echo, "en-GB")).toContain(
      'cal.span.minutes:{\\"n\\":90}',
    );
  });

  it("keeps which end it counts from, and which direction", () => {
    expect(reminderLabel({ trigger: { seconds: -3600, related: "end" } }, echo, "en-GB")).toContain("cal.remind.beforeEnd");
    expect(reminderLabel({ trigger: { seconds: 300, related: "start" } }, echo, "en-GB")).toContain("cal.remind.afterStart");
    expect(reminderLabel({ trigger: { seconds: 0, related: "start" } }, echo, "en-GB")).toBe("cal.remind.atStart");
    expect(reminderLabel({ trigger: { seconds: 0, related: "end" } }, echo, "en-GB")).toBe("cal.remind.atEnd");
  });

  it("writes a fixed instant as its day and time", () => {
    const out = reminderLabel({ trigger: { at: "2026-09-04T18:00" } }, echo, "en-GB");
    expect(out).toContain("cal.remind.on");
    expect(out).toContain("18:00");
    expect(out).toContain("4");
  });
});

describe("isoWeek", () => {
  it("numbers an ordinary week", () => {
    expect(isoWeek("2026-09-03")).toBe(36);
    expect(isoWeek("2026-08-31")).toBe(36);
    expect(isoWeek("2026-09-06")).toBe(36);
  });

  it("puts the last days of December into week 1 when Thursday says so", () => {
    // 2024-12-30 is a Monday whose Thursday is 2 January 2025.
    expect(isoWeek("2024-12-30")).toBe(1);
    expect(isoWeek("2025-01-01")).toBe(1);
  });

  it("puts the first days of January into week 53 when they belong to the old year", () => {
    // 2021-01-01 is a Friday: still week 53 of 2020.
    expect(isoWeek("2021-01-01")).toBe(53);
    expect(isoWeek("2021-01-04")).toBe(1);
  });
});
