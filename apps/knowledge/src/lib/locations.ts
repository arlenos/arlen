/// The Knowledge app's places: the sidebar's navigable locations over the graph.
/// Each is a virtual location key (never a filesystem path) the KG adapter routes.
/// Mirrors the Files app's `locations.ts` split of presentation from data.

/// One explore place in the sidebar.
export interface KnowledgePlace {
  /// The virtual location key the controller navigates to + the adapter lists.
  id: string;
  /// i18n key for the sidebar label.
  labelKey: string;
  /// i18n key for the empty-state line of this location's content.
  emptyKey: string;
}

/// The explore places, in sidebar order (§2 of knowledge-app.md). Capabilities and
/// Capsules are NOT here - both are link-outs to Settings/Privacy, which owns them,
/// handled separately.
export const EXPLORE_PLACES: KnowledgePlace[] = [
  { id: "timeline", labelKey: "k.place.timeline", emptyKey: "k.empty.timeline" },
  { id: "projects", labelKey: "k.place.projects", emptyKey: "k.empty.projects" },
  { id: "searches", labelKey: "k.place.searches", emptyKey: "k.empty.searches" },
  { id: "library", labelKey: "k.place.library", emptyKey: "k.empty.library" },
];

/// The place whose base location a saved search / project refines (search:<q>,
/// project:<id>) still resolves back to its parent place for the header label.
function basePlaceId(location: string): string {
  const scheme = location.split(":")[0] ?? location;
  if (scheme === "search") return "searches";
  if (scheme === "project") return "projects";
  return location;
}

/// The i18n label key for a location (for the content header), or the raw key
/// when the location is unknown.
export function labelKeyFor(location: string): string {
  const place = EXPLORE_PLACES.find((p) => p.id === basePlaceId(location));
  return place?.labelKey ?? "k.title";
}

/// The i18n empty-state key for a location.
export function emptyKeyFor(location: string): string {
  const place = EXPLORE_PLACES.find((p) => p.id === basePlaceId(location));
  return place?.emptyKey ?? "k.empty.timeline";
}
