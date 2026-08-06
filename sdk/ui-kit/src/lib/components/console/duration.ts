/// Format a command's wall-clock duration for a console block: terse and
/// tabular-safe (12ms / 1.2s / 41s / 2m 05s). Extracted from ConsoleBlock so the
/// minute-boundary rounding (the bug-prone part) is unit-testable.
///
/// Sub-second is shown in milliseconds; under ten seconds keeps one decimal;
/// otherwise the value is rounded to whole seconds FIRST and only then split into
/// minutes and seconds, so a value that rounds up across a minute rolls over
/// correctly (119.6s is "2m 00s", not "1m 60s"; 59.6s is "1m 00s", not "60s").
import { get } from "svelte/store";

import { formatDecimal, locale } from "../../i18n";

export function formatDuration(ms: number, loc = get(locale)): string {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  // The one fractional value here. `toFixed` would write "1.5s" to a reader who
  // writes "1,5s"; the rest are whole numbers and carry no separator at these
  // magnitudes.
  if (s < 10) return `${formatDecimal(s, 1, loc)}s`;
  const totalSec = Math.round(s);
  if (totalSec < 60) return `${totalSec}s`;
  const m = Math.floor(totalSec / 60);
  const rest = totalSec % 60;
  return `${m}m ${String(rest).padStart(2, "0")}s`;
}
