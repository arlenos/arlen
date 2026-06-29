/// The confined picker's sidebar places. The picker reaches only real
/// navigable folders (no KG virtual locations), so the set is the
/// conventional user dirs built from the resolved home plus an optional
/// Recent group.
///
/// The folder set is built client-side from `home` for the common XDG
/// layout; a `picker_places` daemon command would resolve localized or
/// user-relocated XDG dirs correctly (flagged in arlen-ui-reports.md).
/// Recent comes from the routed `picker_recent` command and is simply
/// omitted when that command is absent - nothing half-mocks a feed.

import { invoke } from "@tauri-apps/api/core";
import type { Place, PlaceGroup } from "@arlen/ui-kit/components/browser";

interface ConventionalDir {
  label: string;
  icon: string;
  sub: string;
}

/// The conventional XDG user dirs, in sidebar order. Home is the root;
/// the rest hang off it.
const CONVENTIONAL: ConventionalDir[] = [
  { label: "Documents", icon: "documents", sub: "Documents" },
  { label: "Downloads", icon: "downloads", sub: "Downloads" },
  { label: "Pictures", icon: "pictures", sub: "Pictures" },
  { label: "Music", icon: "music", sub: "Music" },
  { label: "Videos", icon: "videos", sub: "Videos" },
  { label: "Desktop", icon: "desktop", sub: "Desktop" },
];

function joinHome(home: string, sub: string): string {
  return `${home.replace(/\/$/, "")}/${sub}`;
}

/// Build the conventional places group from a resolved home path.
export function conventionalPlaces(home: string): PlaceGroup {
  const places: Place[] = [
    { label: "Home", icon: "home", path: home },
    ...CONVENTIONAL.map((d) => ({
      label: d.label,
      icon: d.icon,
      path: joinHome(home, d.sub),
    })),
  ];
  return { label: "Places", places };
}

/// Build the Recent group from the routed picker-side recent feed.
/// Returns null (the group does not render) when there is no recent
/// data or the command is unavailable.
export async function recentGroup(): Promise<PlaceGroup | null> {
  try {
    const recent = await invoke<Place[]>("picker_recent");
    if (!recent || recent.length === 0) return null;
    return {
      label: "Recent",
      // The rail would show identical clock glyphs; keep Recent out of
      // the collapsed icon rail.
      railHidden: true,
      places: recent.map((p) => ({ ...p, icon: "recent" })),
    };
  } catch {
    return null;
  }
}

/// Resolve the home path the picker starts from (the daemon picks the
/// caller's current_folder when valid, else $HOME).
export async function resolveHome(): Promise<string> {
  try {
    return await invoke<string>("resolve_start_dir", { provided: null });
  } catch {
    return "/home";
  }
}
