/// The sidebar's place groups: conventional places and devices from
/// the host, plus the two quiet KG spots (Projects, Searches) —
/// graph-backed lists rendered only when they have content.

import { derived, get, writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { Place, PlaceGroup } from "@arlen/ui-kit/components/browser";

import { t, locale } from "$lib/i18n/messages";

interface Project {
  id: string;
  name: string;
  path: string;
}

interface SavedSearch {
  id: string;
  name: string;
  query: string;
}

/// A volume for the Devices section (mirrors the Rust `MountedDevice`): a mounted
/// one to navigate into, or an unmounted removable drive to mount on click.
interface MountedDevice {
  label: string;
  mountpoint: string;
  device: string;
  removable: boolean;
  mounted: boolean;
  fstype: string;
}

/// Mountpoint -> `/dev` node, so the Devices hover affordance can route to eject
/// (the sidebar only hands `removePlace` a `Place`, keyed by its path).
const deviceNodes = new Map<string, string>();

/// `/dev` nodes of listed-but-unmounted removable drives, keyed by the place path
/// (their `/dev` node, since they have no mountpoint yet). Clicking such a place
/// mounts it first, then navigates into the new mountpoint.
const unmountedDevices = new Set<string>();

/// The loaded groups, whose `label` holds a message KEY for the four section
/// headings the file manager owns. Consumers read [`placeGroups`], which resolves
/// them; the raw store is what the loader writes.
const placeGroupsRaw = writable<PlaceGroup[]>([]);

/// The sidebar's groups with their headings translated.
///
/// Derived rather than translated in the loader: the loader runs once when the
/// sidebar mounts, so translating there would pin the headings to whichever
/// locale was active then. The place entries inside each group keep their real
/// names - a device or bookmark label is data, not chrome.
export const placeGroups = derived([placeGroupsRaw, t], ([$groups, $t]) =>
  $groups.map((g) => ({ ...g, label: $t(g.label) })),
);

/// Pin a folder to the sidebar and refresh the groups.
export async function addBookmark(path: string): Promise<void> {
  try {
    await invoke("files_bookmark_add", { path });
  } catch {
    return;
  }
  await loadPlaces();
}

/// Unpin a folder.
export async function removeBookmark(path: string): Promise<void> {
  try {
    await invoke("files_bookmark_remove", { path });
  } catch {
    return;
  }
  await loadPlaces();
}
/// The Devices/Bookmarks hover-remove affordance: a removable DEVICE ejects
/// (unmount + power-off the drive via udisks); any other removable place (a user
/// bookmark) unpins. One handler so the sidebar's single affordance does the
/// right thing per entry type.
export async function removePlace(place: Place): Promise<void> {
  const device = deviceNodes.get(place.path);
  if (device) {
    try {
      await invoke("files_eject", { device });
    } catch {
      // Busy / polkit-refused: leave the device listed.
    }
    await loadPlaces();
  } else {
    await removeBookmark(place.path);
  }
}

/// Navigate to a place. An unmounted removable drive is mounted first (udisks),
/// then the now-mounted volume is opened; everything else navigates directly.
/// A mount that fails (busy / polkit-refused) stays put rather than opening the
/// bare `/dev` node.
export async function navigatePlace(
  place: Place,
  navigate: (path: string) => void,
): Promise<void> {
  if (unmountedDevices.has(place.path)) {
    try {
      await invoke("files_mount", { device: place.path });
    } catch {
      return;
    }
    await loadPlaces();
    for (const [mountpoint, device] of deviceNodes) {
      if (device === place.path) {
        navigate(mountpoint);
        return;
      }
    }
    return;
  }
  navigate(place.path);
}

export const savedSearches = writable<SavedSearch[]>([]);

/// The home place's path; the breadcrumb collapses it to "Home".
export const homePath = writable("/home");

/// Resolve a group heading before it is handed to the kit.
///
/// `PlacesSidebar` renders `group.label` verbatim, as it must: the kit cannot
/// resolve an app's messages. These four were message ids going straight into
/// that, so the sidebar showed `f.places.places` as a heading. Resolved here,
/// the same way `sysOptions` does it in Settings.
const tr = (id: string) => get(t)(id);

/// Resolve a place's name where the backend sent an id.
///
/// The XDG user dirs are ours to name; a volume or a bookmarked folder carries
/// its own name and arrives without a key.
const named = (p: Place): Place =>
  p.labelKey ? { ...p, label: tr(p.labelKey) } : p;

/// Reload when the language changes.
///
/// The headings and the standard places are resolved as they are pushed, which
/// freezes whichever language was current at that moment. `initArlenLocale()` is
/// a round-trip, so on a German desktop the first load beat it and the sidebar
/// came up with "PLACES" in English next to "GERÄTE" in German - one heading each
/// side of the race, in the same list. Re-running on a change settles both, and
/// gives a live switch for free.
let known: string | null = null;
locale.subscribe((tag) => {
  if (known === null) {
    known = tag;
    return;
  }
  if (tag !== known) {
    known = tag;
    void loadPlaces();
  }
});

export async function loadPlaces(): Promise<void> {
  const groups: PlaceGroup[] = [];
  try {
    const places = await invoke<{ orte: Place[]; geraete: Place[] }>("files_places");
    const home = places.orte.find((p) => p.icon === "home");
    if (home) homePath.set(home.path);
    groups.push({ label: tr("f.places.places"), places: places.orte.map(named) });
    // Devices: the host's base entry (System) plus the real mounted volumes
    // from lsblk (removable drives + extra data mounts). Removable ones carry
    // the hover affordance, routed to eject by `removePlace` via deviceNodes.
    deviceNodes.clear();
    unmountedDevices.clear();
    const devicePlaces: Place[] = [];
    try {
      const devs = await invoke<MountedDevice[]>("files_devices");
      for (const d of devs) {
        if (d.mounted) {
          // A mounted volume: navigate to its mountpoint; removable ones carry
          // the eject affordance (routed through deviceNodes by removePlace).
          deviceNodes.set(d.mountpoint, d.device);
          devicePlaces.push({
            label: d.label,
            icon: "drive",
            path: d.mountpoint,
            removable: d.removable,
          });
        } else {
          // An unmounted removable drive: the place path is its /dev node and a
          // click mounts-then-navigates. No eject affordance (nothing to eject).
          unmountedDevices.add(d.device);
          devicePlaces.push({
            label: d.label,
            icon: "drive",
            path: d.device,
            removable: false,
          });
        }
      }
    } catch {
      // lsblk unavailable: just the host base entries.
    }
    groups.push({
      label: tr("f.places.devices"),
      places: [...places.geraete.map(named), ...devicePlaces],
    });
  } catch {
    // Unreachable backend: the sidebar stays empty rather than fake.
  }
  try {
    const bookmarks = await invoke<Place[]>("files_bookmarks");
    groups.push({
      label: tr("f.places.bookmarks"),
      railHidden: true,
      places: bookmarks.map((b) => ({ ...b, removable: true })),
    });
  } catch {
    // No bookmark store: the group does not render.
  }
  try {
    const projects = await invoke<Project[]>("files_projects");
    groups.push({
      label: tr("f.places.projects"),
      // The rail would show two identical glyphs; the group only
      // makes sense expanded.
      railHidden: true,
      places: projects.map((p) => ({
        label: p.name,
        icon: "project",
        path: p.path,
      })),
    });
  } catch {
    // No graph yet: the group simply does not render.
  }
  placeGroupsRaw.set(groups);

  try {
    savedSearches.set(await invoke<SavedSearch[]>("files_saved_searches"));
  } catch {
    savedSearches.set([]);
  }
}
