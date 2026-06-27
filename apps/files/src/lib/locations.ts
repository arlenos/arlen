/// Host-owned label text for a virtual KG location (recent / trash / project:
/// / search:), rendered by the kit breadcrumb as a single non-navigable name
/// crumb. The kit owns the path-vs-name STRUCTURE (`locationCrumbs`); this owns
/// the TEXT, so the kit stays i18n-neutral. A real path returns unchanged.
import {
  type PlaceGroup,
  type ColumnSpec,
  DEFAULT_COLUMNS,
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
  return path;
}

/// The column set a location shows. A virtual location swaps Size for the item's
/// home folder (its members are scattered) and relabels the time column; a real
/// folder keeps the default Name | Size | Modified.
export function columnsFor(path: string): ColumnSpec {
  if (path === "trash") {
    return { middle: "location", middleLabel: "Original location", timeLabel: "Deleted" };
  }
  if (path === "recent") {
    return { middle: "location", middleLabel: "Location", timeLabel: "Last accessed" };
  }
  if (path.startsWith("project:") || path.startsWith("search:")) {
    return { middle: "location", middleLabel: "Location", timeLabel: "Modified" };
  }
  return DEFAULT_COLUMNS;
}
