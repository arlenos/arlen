/// The rules this window applies to a row, out where they can be tested.
///
/// All three lived inside `+page.svelte` as private functions. Two of them are
/// about somebody's clock and one is about somebody's week, and none could be
/// exercised without rendering the component. The window's LOOK is another
/// lane's to redo; these rules are not, so they should not be in the file that
/// is going away.

import type { Reminder } from "$lib/stores/calendar";

/// A translator: the message id and its values in, the sentence out.
export type Translate = (key: string, values?: Record<string, unknown>) => string;

const DAY_KEY: Record<string, string> = {
  mon: "cal.dayMon",
  tue: "cal.dayTue",
  wed: "cal.dayWed",
  thu: "cal.dayThu",
  fri: "cal.dayFri",
  sat: "cal.daySat",
  sun: "cal.daySun",
};

const EVERY_KEY: Record<string, string> = {
  daily: "cal.everyDaily",
  weekly: "cal.everyWeekly",
  monthly: "cal.everyMonthly",
  yearly: "cal.everyYearly",
};

/// Whether `date` (a `YYYY-MM-DD` in the event's own local terms) is today.
///
/// Compared as the reader's own LOCAL date rather than through UTC, because
/// "today" is a fact about the reader's clock. `now` is a parameter for the same
/// reason: a function that reads the wall clock itself can only be tested by
/// waiting, and the interesting cases are the ones around midnight and the turn
/// of a month, which nobody is going to sit up for.
export function isToday(date: string, now: Date): boolean {
  const local = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(
    now.getDate(),
  ).padStart(2, "0")}`;
  return date === local;
}

/// A day heading, in the reader's language.
///
/// Built from the parts rather than by parsing the string as a date, because
/// `new Date("2026-08-21")` is UTC midnight and would show the day before to
/// anybody west of Greenwich.
export function dayLabel(date: string, loc: string): string {
  const [y, m, d] = date.split("-").map(Number);
  return new Intl.DateTimeFormat(loc, {
    weekday: "long",
    day: "numeric",
    month: "long",
  }).format(new Date(y, m - 1, d));
}

/// What the repeat chip says.
///
/// "Repeats" alone was true of a standup every weekday and of a birthday every
/// year, which is the same as saying nothing. The event carries the frequency,
/// the interval and the weekdays as keys, and the sentence is written from them
/// so it is written in the reader's language.
///
/// A rule the calendar refuses carries no frequency, and then the chip goes back
/// to the bare word: better vague than wrong about somebody's week.
export function repeatLabel(
  e: { every: string | null; every_n: number; on_days: string[] },
  t: Translate,
): string {
  const key = e.every ? EVERY_KEY[e.every] : undefined;
  if (!key) return t("cal.repeats");
  const every = t(key, { n: e.every_n });
  if (e.on_days.length === 0) return every;
  const days = e.on_days.map((d) => (DAY_KEY[d] ? t(DAY_KEY[d]) : d)).join(", ");
  return t("cal.onDays", { every, days });
}

/// What a reminder line says: "10 minutes before", "1 hour before the end",
/// "On Friday, 4 September, 18:00". Written from the trigger's parts rather
/// than carried as a sentence, so it is in the reader's language; a span is
/// said in the largest unit that divides it (a day, not 1440 minutes).
export function reminderLabel(r: Reminder, t: Translate, loc: string): string {
  if ("at" in r.trigger) {
    const [date, time] = r.trigger.at.split("T");
    const when = time ? `${dayLabel(date, loc)}, ${time.slice(0, 5)}` : dayLabel(date, loc);
    return t("cal.remind.on", { when });
  }
  const { seconds, related } = r.trigger;
  if (seconds === 0) return t(related === "end" ? "cal.remind.atEnd" : "cal.remind.atStart");
  const span = spanLabel(Math.abs(seconds), t);
  const before = seconds < 0;
  const key =
    related === "end"
      ? before
        ? "cal.remind.beforeEnd"
        : "cal.remind.afterEnd"
      : before
        ? "cal.remind.beforeStart"
        : "cal.remind.afterStart";
  return t(key, { span });
}

/// A duration in the largest unit that divides it exactly.
export function spanLabel(seconds: number, t: Translate): string {
  if (seconds % 86_400 === 0) return t("cal.span.days", { n: seconds / 86_400 });
  if (seconds % 3_600 === 0) return t("cal.span.hours", { n: seconds / 3_600 });
  return t("cal.span.minutes", { n: Math.round(seconds / 60) });
}

/// The ISO 8601 week number of a `YYYY-MM-DD`: weeks start on Monday and
/// week 1 is the one holding the year's first Thursday, which is why the
/// last days of December can be week 1 and the first days of January week 53.
/// Computed in UTC from the parts so the reader's zone cannot shift the day.
export function isoWeek(date: string): number {
  const [y, m, d] = date.split("-").map(Number);
  const at = new Date(Date.UTC(y, m - 1, d));
  const weekday = at.getUTCDay() || 7;
  at.setUTCDate(at.getUTCDate() + 4 - weekday);
  const yearStart = Date.UTC(at.getUTCFullYear(), 0, 1);
  return Math.ceil(((at.getTime() - yearStart) / 86_400_000 + 1) / 7);
}

/// The bar title for a month: "August 2026", in the reader's language.
export function monthTitle(date: string, loc: string): string {
  const [y, m] = date.split("-").map(Number);
  return new Intl.DateTimeFormat(loc, { month: "long", year: "numeric" }).format(new Date(y, m - 1, 1));
}

/// The bar title for a week: its span, month names only where they change.
export function weekTitle(monday: string, loc: string): string {
  const [y, m, d] = monday.split("-").map(Number);
  const start = new Date(y, m - 1, d);
  const end = new Date(y, m - 1, d + 6);
  return new Intl.DateTimeFormat(loc, { day: "numeric", month: "short", year: "numeric" }).formatRange(start, end);
}

/// The bar title for a day: the full date.
export function dayTitle(date: string, loc: string): string {
  const [y, m, d] = date.split("-").map(Number);
  return new Intl.DateTimeFormat(loc, {
    weekday: "long",
    day: "numeric",
    month: "long",
    year: "numeric",
  }).format(new Date(y, m - 1, d));
}
