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
