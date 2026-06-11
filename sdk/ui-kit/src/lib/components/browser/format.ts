/// Display formatting for the browser archetype: sizes and modified
/// times the way a file manager speaks them — short, lay-readable,
/// no internal units.

/// 0 B / 18 KB / 2.4 MB / 4.0 GB.
export function formatSize(bytes: number | null): string {
  if (bytes === null) return "";
  if (bytes < 1000) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes;
  for (const u of units) {
    v /= 1000;
    if (v < 1000) return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${u}`;
  }
  return `${Math.round(v)} PB`;
}

/// just now / 12 min ago / 3 hours ago / yesterday / 4 days ago /
/// May 12 / May 12, 2025. `now` is injectable for stable screenshots.
export function formatModified(unix: number | null, now = Date.now() / 1000): string {
  if (unix === null) return "";
  const diff = Math.max(0, now - unix);
  if (diff < 90) return "just now";
  if (diff < 3600) return `${Math.round(diff / 60)} min ago`;
  if (diff < 2 * 86400) {
    const h = Math.round(diff / 3600);
    if (h <= 23) return h === 1 ? "1 hour ago" : `${h} hours ago`;
    return "yesterday";
  }
  if (diff < 14 * 86400) return `${Math.round(diff / 86400)} days ago`;
  const d = new Date(unix * 1000);
  const sameYear = new Date(now * 1000).getFullYear() === d.getFullYear();
  const month = d.toLocaleString("en", { month: "short" });
  return sameYear
    ? `${month} ${d.getDate()}`
    : `${month} ${d.getDate()}, ${d.getFullYear()}`;
}
