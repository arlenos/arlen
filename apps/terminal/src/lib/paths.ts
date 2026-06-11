/// Path display helpers. The contract ships absolute paths; these are
/// presentation only and the single place the home-shortening lives.

/// `/home/<user>` becomes `~`, `/home/<user>/x` becomes `~/x`.
export function tildify(path: string): string {
  const m = path.match(/^\/home\/[^/]+(\/.*)?$/);
  if (!m) return path;
  return "~" + (m[1] ?? "");
}

/// Compact form for sidebar rows: home-shortened, then at most the
/// last two segments (`~`, `~/Downloads`, `Repositories/arlen`).
export function shortPath(path: string): string {
  const t = tildify(path);
  const parts = t.split("/").filter((x) => x.length > 0);
  if (parts.length === 0) return "/";
  if (parts.length <= 2) return parts.join("/");
  return parts[parts.length - 2] + "/" + parts[parts.length - 1];
}
