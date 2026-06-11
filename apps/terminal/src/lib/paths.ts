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

/// Row display form: the full home-shortened path when it fits,
/// otherwise whole leading segments give way to a `…/` prefix — the
/// leaf always survives, and every row speaks the same shape
/// (`~/Repositories/arlen`, `…/arlen/docs`). Mono rendering makes
/// the character budget exact.
export function displayPath(path: string, maxChars = 24): string {
  const t = tildify(path);
  if (t.length <= maxChars) return t;
  const parts = t.split("/").filter((x) => x.length > 0);
  while (parts.length > 1 && ("…/" + parts.join("/")).length > maxChars + 1) {
    parts.shift();
  }
  return "…/" + parts.join("/");
}
