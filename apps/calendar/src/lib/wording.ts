/// The rules this window applies to a row, out where they can be tested.
///
/// All three lived inside `+page.svelte` as private functions. Two of them are
/// about somebody's clock and one is about somebody's week, and none could be
/// exercised without rendering the component. The window's LOOK is another
/// lane's to redo; these rules are not, so they should not be in the file that
/// is going away.

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
