/// The sidebar's place groups: conventional places and devices from
/// the host, plus the two quiet KG spots (Projects, Searches) —
/// graph-backed lists rendered only when they have content.

import { writable } from "svelte/store";
import { invoke } from "@tauri-apps/api/core";
import type { Place, PlaceGroup } from "@arlen/ui-kit/components/browser";

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

export const placeGroups = writable<PlaceGroup[]>([]);
export const savedSearches = writable<SavedSearch[]>([]);

/// The home place's path; the breadcrumb collapses it to "Home".
export const homePath = writable("/home");

export async function loadPlaces(): Promise<void> {
  const groups: PlaceGroup[] = [];
  try {
    const places = await invoke<{ orte: Place[]; geraete: Place[] }>("files_places");
    const home = places.orte.find((p) => p.icon === "home");
    if (home) homePath.set(home.path);
    groups.push({ label: "Places", places: places.orte });
    groups.push({ label: "Devices", places: places.geraete });
  } catch {
    // Unreachable backend: the sidebar stays empty rather than fake.
  }
  try {
    const projects = await invoke<Project[]>("files_projects");
    groups.push({
      label: "Projects",
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
  placeGroups.set(groups);

  try {
    savedSearches.set(await invoke<SavedSearch[]>("files_saved_searches"));
  } catch {
    savedSearches.set([]);
  }
}
