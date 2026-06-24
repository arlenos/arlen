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

/// Prompt form: the home-shortened path with deep ancestors abbreviated to
/// their first character, the current folder (the anchor) and its immediate
/// parent kept whole. `~/Repositories/arlen/apps/terminal` becomes
/// `~/R/a/apps/terminal`; a short path (`~/Repositories/arlen`) is left as is.
/// Returns the dim prefix and the bright anchor separately so the prompt can
/// weight them the way p10k does (faint trail, solid current folder).
export function collapsePath(path: string): { prefix: string; anchor: string } {
  const t = tildify(path);
  const parts = t.split("/");
  const anchor = parts.pop() ?? "";
  const last = parts.length - 1;
  const shown = parts.map((p, i) => {
    if (p === "" || p === "~") return p;
    return i >= last ? p : p.slice(0, 1);
  });
  const prefix = shown.join("/");
  return { prefix: prefix ? prefix + "/" : "", anchor: anchor || "/" };
}

/// Row display form: the full home-shortened path when it fits,
/// otherwise whole leading segments give way to a `…/` prefix — the
/// leaf always survives, and every row speaks the same shape
/// (`~/Repositories/arlen`, `…/arlen/docs`). Mono rendering makes
/// the character budget exact.
export function displayPath(path: string, maxChars = 26): string {
  const t = tildify(path);
  if (t.length <= maxChars) return t;
  const parts = t.split("/").filter((x) => x.length > 0);
  while (parts.length > 1 && ("…/" + parts.join("/")).length > maxChars) {
    parts.shift();
  }
  const candidate = "…/" + parts.join("/");
  // An ellipsis that does not actually shorten anything only loses
  // information; keep the honest full form then.
  return candidate.length >= t.length ? t : candidate;
}
