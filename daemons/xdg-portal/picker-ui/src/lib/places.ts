/// The confined picker's sidebar places. The picker reaches only real
/// navigable folders (no KG virtual locations), so the set is the
/// conventional user dirs built from the resolved home plus an optional
/// Recent group.
///
/// The folder set is built client-side from `home` for the common XDG
/// layout; a `picker_places` daemon command would resolve localized or
/// user-relocated XDG dirs correctly (flagged in arlen-ui-reports.md).
/// Recent comes from the routed `picker_recent` command, which NOTHING
/// REGISTERS today, so on a real machine the group is always omitted and the
/// sidebar is Places only. Only the mock in `routes/_pickertest` answers it.
/// The omission is the right behaviour; the absence of a producer is the
/// reason it always happens.

import { invoke } from "@tauri-apps/api/core";
import type { Place, PlaceGroup } from "@arlen/ui-kit/components/browser";

/// Write one message. The caller passes it in: this module has no locale of its
/// own, and a place list built at startup in whatever language was current would
/// keep it through a switch.
type Write = (key: string) => string;

interface ConventionalDir {
  /// The message key for what a reader sees.
  key: string;
  icon: string;
  /// The folder ON DISK, which keeps its own name whatever the reader's
  /// language is. Translating this would build a path that does not exist.
  sub: string;
}

/// The conventional XDG user dirs, in sidebar order. Home is the root;
/// the rest hang off it.
const CONVENTIONAL: ConventionalDir[] = [
  { key: "p.place.documents", icon: "documents", sub: "Documents" },
  { key: "p.place.downloads", icon: "downloads", sub: "Downloads" },
  { key: "p.place.pictures", icon: "pictures", sub: "Pictures" },
  { key: "p.place.music", icon: "music", sub: "Music" },
  { key: "p.place.videos", icon: "videos", sub: "Videos" },
  { key: "p.place.desktop", icon: "desktop", sub: "Desktop" },
];

function joinHome(home: string, sub: string): string {
  return `${home.replace(/\/$/, "")}/${sub}`;
}

/// Build the conventional places group from a resolved home path.
export function conventionalPlaces(home: string, write: Write): PlaceGroup {
  const places: Place[] = [
    { label: write("p.place.home"), icon: "home", path: home },
    ...CONVENTIONAL.map((d) => ({
      label: write(d.key),
      icon: d.icon,
      path: joinHome(home, d.sub),
    })),
  ];
  return { label: write("p.places.places"), places };
}

/// Fetch the recent places from the routed picker-side recent feed.
/// Returns null (the group does not render) when there is no recent
/// data or the command is unavailable.
export async function recentPlaces(): Promise<Place[] | null> {
  try {
    const recent = await invoke<Place[]>("picker_recent");
    if (!recent || recent.length === 0) return null;
    return recent.map((p) => ({ ...p, icon: "recent" }));
  } catch {
    return null;
  }
}

/// Wrap the recent places in their group. Separate from the fetch so the group
/// can be rebuilt in a new language without asking the daemon again.
export function recentGroup(places: Place[], write: Write): PlaceGroup {
  return {
    label: write("p.places.recent"),
    // The rail would show identical clock glyphs; keep Recent out of
    // the collapsed icon rail.
    railHidden: true,
    places,
  };
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
