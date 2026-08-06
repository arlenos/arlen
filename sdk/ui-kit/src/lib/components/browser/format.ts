import { get } from "svelte/store";

import { locale } from "../../i18n";

/// Display formatting for the browser archetype: sizes and modified
/// times the way a file manager speaks them — short, lay-readable,
/// no internal units.

/// 0 B / 18 KB / 2.4 MB / 4.0 GB, with the reader's decimal mark.
///
/// `toFixed` always writes a point, so this said "2.4 MB" on a German desktop
/// where the number wants a comma - the same shape as the hardcoded month name
/// described below, one line up from the comment that records it. `Intl` knows
/// the separator for every locale the platform does.
export function formatSize(bytes: number | null, loc = get(locale)): string {
  if (bytes === null) return "";
  if (bytes < 1000) return `${decimals(bytes, 0, loc)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes;
  for (const u of units) {
    v /= 1000;
    if (v < 1000) return `${decimals(v, v < 10 ? 1 : 0, loc)} ${u}`;
  }
  return `${decimals(v, 0, loc)} PB`;
}

/// A number with exactly `digits` decimals, in `loc`.
function decimals(value: number, digits: number, loc: string): string {
  return new Intl.NumberFormat(loc, {
    minimumFractionDigits: digits,
    maximumFractionDigits: digits,
  }).format(value);
}

/// "now" / "12 minutes ago" / "yesterday" / "4 days ago" / "12 May" /
/// "12 May 2025", in the caller's language. `now` is injectable for stable
/// screenshots.
///
/// Every word here used to be written in English, including the month name via a
/// hardcoded `toLocaleString("en")` and the date order `May 12, 2025`, which is one
/// convention out of many - German wants `12. Mai`. The plural was a ternary, which
/// is the assumption that a language has two forms and splits them at one.
///
/// So none of it is written here any more. `Intl.RelativeTimeFormat` carries the
/// wording, the plural rules and the idiomatic "yesterday" for every locale the
/// platform knows, and `Intl.DateTimeFormat` carries the field order. That is the
/// same reason the weekday strip stopped shipping a list of day names: this data
/// has a canonical source, and ours would be a worse copy of it in one language.
export function formatModified(
  unix: number | null,
  now = Date.now() / 1000,
  loc = get(locale),
): string {
  if (unix === null) return "";
  const diff = Math.max(0, now - unix);
  // `numeric: "auto"` is what turns -1 day into "yesterday" rather than "1 day
  // ago", per locale, which is the phrasing a file manager wants.
  const rel = new Intl.RelativeTimeFormat(loc, { numeric: "auto" });

  if (diff < 90) return rel.format(0, "second");
  if (diff < 3600) return rel.format(-Math.round(diff / 60), "minute");
  if (diff < 2 * 86400) {
    const h = Math.round(diff / 3600);
    return h <= 23 ? rel.format(-h, "hour") : rel.format(-1, "day");
  }
  if (diff < 14 * 86400) return rel.format(-Math.round(diff / 86400), "day");

  const d = new Date(unix * 1000);
  const sameYear = new Date(now * 1000).getFullYear() === d.getFullYear();
  return new Intl.DateTimeFormat(loc, {
    day: "numeric",
    month: "short",
    ...(sameYear ? {} : { year: "numeric" }),
  }).format(d);
}
