/// Pure breadcrumb decomposition, the TS mirror of core::breadcrumb:
/// `/home/x/p` becomes `/`, `home`, `x`, `p`, each crumb carrying the
/// absolute path navigating to it produces. `.`/`..` segments are
/// ignored (the path bar shows canonical locations, not relative
/// steps). Pure logic — an IPC round trip for this would be waste.

import type { Crumb } from "./types";

export function breadcrumb(path: string): Crumb[] {
  const crumbs: Crumb[] = [];
  if (path.startsWith("/")) {
    crumbs.push({ name: "/", path: "/" });
  }
  let acc = "";
  for (const seg of path.split("/")) {
    if (seg.length === 0 || seg === "." || seg === "..") continue;
    acc += "/" + seg;
    crumbs.push({ name: seg, path: acc });
  }
  return crumbs;
}

/// Whether a browser location is a VIRTUAL location rather than a real filesystem
/// path. A real path is absolute (`/home/x`); a virtual location is the host's
/// opaque key for a listing whose contents are a query, not a directory - Recent,
/// Trash, `project:<id>`, `search:<query>` (item 12). The browser routes both
/// through the same adapter `list()`, but they decompose into crumbs differently
/// (a real path is a navigable hierarchy, a virtual location a single name).
export function isVirtualLocation(location: string): boolean {
  return location.length > 0 && !location.startsWith("/");
}

/// Decompose a browser location into breadcrumb segments. A real filesystem path
/// becomes its navigable hierarchy (see [`breadcrumb`]); a VIRTUAL location (item
/// 12: Recent / Trash / `project:<id>` / `search:<query>`) becomes ONE
/// non-navigable NAME crumb - a virtual location is a single place, not a path
/// hierarchy, so there are no parent steps to climb. `label` is the displayed name
/// (the host owns the text: a project's name, the translated "Recent"/"Trash", the
/// search terms); the crumb's `path` is the location itself, so a click round-trips
/// it back through the adapter unchanged. Pure, the structure decision only; the
/// per-location columns/toolbar/actions are the host's presentation.
export function locationCrumbs(location: string, label: string): Crumb[] {
  if (isVirtualLocation(location)) {
    return [{ name: label, path: location }];
  }
  return breadcrumb(location);
}
