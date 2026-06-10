/// Relative wall-clock formatting for ledger timestamps. Entries carry
/// microsecond timestamps (audit ledger resolution); display is coarse
/// ("8 min ago"), falling back to a date beyond a week.

/// Format a microsecond timestamp relative to `nowMs` (injectable for tests;
/// defaults to the wall clock).
export function relativeTime(micros: number, nowMs: number = Date.now()): string {
  const then = micros / 1000;
  const diffSec = Math.max(0, (nowMs - then) / 1000);
  if (diffSec < 45) return "just now";
  if (diffSec < 90) return "a minute ago";
  const min = Math.round(diffSec / 60);
  if (min < 60) return `${min} min ago`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr} h ago`;
  const day = Math.round(hr / 24);
  if (day < 7) return `${day} d ago`;
  return new Date(then).toLocaleDateString();
}
