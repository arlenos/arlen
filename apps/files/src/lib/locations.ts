/// Host-owned label text for a virtual KG location (recent / trash / project:
/// / search: / facet:), rendered by the kit breadcrumb as a single
/// non-navigable name crumb. The kit owns the path-vs-name STRUCTURE
/// (`locationCrumbs`); this owns the TEXT, so the kit stays i18n-neutral. A real
/// path returns unchanged.
import {
  type PlaceGroup,
  type ColumnSpec,
} from "@arlen/ui-kit/components/browser";

/// Translate a location key into its display label. `groups` lets a
/// `project:<id>` resolve to the project's place label; without it (or on a
/// miss) the bare id is shown.
export function locationLabel(path: string, groups: PlaceGroup[] = []): string {
  if (path === "recent") return "Recent";
  if (path === "trash") return "Trash";
  if (path.startsWith("search:")) {
    const q = path.slice("search:".length).trim();
    return q ? `Search: ${q}` : "Search";
  }
  if (path.startsWith("project:")) {
    for (const g of groups) {
      const hit = g.places.find((pl) => pl.path === path);
      if (hit) return hit.label;
    }
    return path.slice("project:".length);
  }
  if (path.startsWith("facet:")) {
    // A saved Smart Folder resolves to its name; an ad hoc filter reads as
    // "Filtered" (the chips above the listing carry the specifics).
    for (const g of groups) {
      const hit = g.places.find((pl) => pl.path === path);
      if (hit) return hit.label;
    }
    return "Filtered";
  }
  return path;
}

/// The column set a location shows. A virtual location swaps Size for the item's
/// home folder (its members are scattered) and relabels the time column; a real
/// folder keeps the default Name | Size | Modified.
/// The labels are i18n KEYS (not display text); the caller resolves them through the
/// catalog so the column headers follow the locale.
export function columnsFor(path: string): ColumnSpec {
  if (path === "trash") {
    return { middle: "location", middleLabel: "f.col.originalLocation", timeLabel: "f.col.deleted" };
  }
  if (path === "recent") {
    return { middle: "location", middleLabel: "f.col.location", timeLabel: "f.col.lastAccessed" };
  }
  if (
    path.startsWith("project:") ||
    path.startsWith("search:") ||
    path.startsWith("facet:")
  ) {
    return { middle: "location", middleLabel: "f.col.location", timeLabel: "f.col.modified" };
  }
  return { middle: "size", middleLabel: "f.col.size", timeLabel: "f.col.modified" };
}

/// The i18n key for the message a location shows when it has no items. A virtual
/// location speaks for itself rather than the generic folder phrasing. The caller
/// resolves the key through the catalog, so the label follows the locale.
export function emptyLabelFor(path: string): string {
  if (path === "trash") return "f.empty.trash";
  if (path === "recent") return "f.empty.recent";
  if (path.startsWith("project:")) return "f.empty.project";
  if (path.startsWith("search:")) return "f.empty.search";
  if (path.startsWith("facet:")) return "f.empty.facet";
  return "f.empty.folder";
}
