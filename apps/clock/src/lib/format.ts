/// Display formatting for the clock surfaces. Times format through Intl off
/// the shared locale store; durations are digit groups with tabular rendering
/// left to CSS.

/// "mm:ss" under an hour, "h:mm:ss" above.
export function fmtDuration(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${two(m)}:${two(s)}`;
}

/// Stopwatch form with centiseconds: "mm:ss.cc" (hours prepend when reached).
export function fmtStopwatch(ms: number): string {
  const cs = Math.floor((Math.max(0, ms) % 1000) / 10);
  return `${fmtDuration(ms)}.${String(cs).padStart(2, "0")}`;
}

/// A short relative form for "rings in": "in 7 h 12 min", "in 3 min".
export function fmtIn(ms: number, locale: string): string {
  const min = Math.max(1, Math.round(ms / 60_000));
  const rtf = new Intl.RelativeTimeFormat(locale, { style: "short" });
  if (min >= 60 * 24) return rtf.format(Math.round(min / (60 * 24)), "day");
  if (min >= 60) return rtf.format(Math.round(min / 60), "hour");
  return rtf.format(min, "minute");
}

/// The wall-clock time in a zone, in the user's locale.
export function zoneTime(zone: string, locale: string, now: number): string {
  return new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit", timeZone: zone }).format(now);
}

/// Whole-hour offset of a zone against the local one at `now` (can be
/// half-hour zones; render with one decimal only when fractional).
export function zoneOffsetHours(zone: string, now: number): number {
  const at = (tz?: string) =>
    new Date(new Intl.DateTimeFormat("en-US", { timeZone: tz, hour12: false, year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(now)).getTime();
  return Math.round(((at(zone) - at(undefined)) / 3600_000) * 2) / 2;
}

/// Which calendar day the zone is on relative to the local one: -1, 0 or 1.
export function zoneDayShift(zone: string, now: number): number {
  const day = (tz?: string) => new Intl.DateTimeFormat("en-CA", { timeZone: tz, day: "2-digit" }).format(now);
  const local = day(undefined);
  const there = day(zone);
  if (local === there) return 0;
  const shifted = new Date(now + 86_400_000);
  return new Intl.DateTimeFormat("en-CA", { day: "2-digit" }).format(shifted) === there ? 1 : -1;
}

/// The repeat-days summary for an alarm row ("Mon-Fri", "Sat, Sun"), from the
/// kit DaysPicker convention (0 = Monday), localized via Intl weekday names.
export function fmtDays(days: number[], locale: string, everyDay: string, once: string): string {
  if (days.length === 0) return once;
  if (days.length === 7) return everyDay;
  const names: string[] = [];
  // 2024-01-01 was a Monday; index 0 maps to it.
  for (const d of [...days].sort((a, b) => a - b)) {
    names.push(new Intl.DateTimeFormat(locale, { weekday: "short" }).format(new Date(2024, 0, 1 + d)));
  }
  const sorted = [...days].sort((a, b) => a - b);
  const contiguous = sorted.length > 2 && sorted.every((d, i) => i === 0 || d === sorted[i - 1] + 1);
  return contiguous ? `${names[0]}-${names[names.length - 1]}` : names.join(", ");
}
