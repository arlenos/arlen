/// The Browser archetype's shared types, mirroring the public Rust of
/// `apps/files/core` field for field (serde lowercase enums). Hosts
/// (the FM app, the xdg picker, the settings picker) re-export from
/// here; the contract changes in the core crate first, never here.

/// What an entry is.
export type EntryKind = "directory" | "file" | "symlink" | "other";

/// One listing entry (core::FileEntry).
export interface FileEntry {
  name: string;
  kind: EntryKind;
  /// Bytes; null for directories.
  size: number | null;
  /// Seconds since epoch; null when unreadable.
  modified_unix: number | null;
  is_hidden: boolean;
  readonly: boolean;
  symlink_target: string | null;
  /// The entry's own absolute path, set ONLY for a virtual-location listing
  /// (Recent / Trash / project / search) where each entry lives in a different
  /// directory, not under one browsed folder - the bridge back to the item's
  /// real home (the "Reveal in containing folder" action every virtual location
  /// offers). null/absent for a normal folder listing. For Trash this is the
  /// ORIGINAL path. Additive + backward-compat.
  full_path?: string | null;
  /// An opaque per-entry token a location-specific action needs that the path
  /// alone cannot supply - currently the Trash trashed name, which Restore /
  /// Delete-forever pass back. null/absent for any normal entry. Additive.
  restore_token?: string | null;
}

/// One breadcrumb segment (core::Crumb).
export interface Crumb {
  name: string;
  path: string;
}

/// The sort key (core::SortKey).
export type SortKey = "name" | "size" | "modified" | "type";

/// A full sort specification, the second adapter argument.
export interface SortSpec {
  key: SortKey;
  foldersFirst: boolean;
  ascending: boolean;
}

/// The one seam between the browser and its host: how a directory is
/// listed. Paths are host-opaque strings; the browser only ever
/// navigates between values the host (or the listing itself) handed
/// out, and never composes paths above its `root`.
export interface BrowserAdapter {
  list(path: string, sort: SortSpec): Promise<FileEntry[]>;
  /// A renderable URL (data: or asset:) for the entry's thumbnail, or
  /// null when the host has none — the tile keeps its icon. Called
  /// lazily per visible grid tile, never on the listing path.
  thumbnail?(path: string, entry: FileEntry): Promise<string | null>;
}

/// A navigable place in the sidebar (label + icon key + path). The
/// icon key resolves through the kit icon map; `offline` renders the
/// gray status dot.
export interface Place {
  label: string;
  icon: string;
  path: string;
  offline?: boolean;
  /// The row offers a quiet hover remove (user bookmarks).
  removable?: boolean;
}

/// One sidebar group of places.
export interface PlaceGroup {
  label: string;
  places: Place[];
  /// Hide the group in the collapsed icon rail (for groups whose
  /// icons are not distinctive enough to stand alone).
  railHidden?: boolean;
}

/// Join a directory path and an entry name (the kit owns this
/// convention; core entries are name-only).
export function joinPath(dir: string, name: string): string {
  return dir.endsWith("/") ? dir + name : dir + "/" + name;
}

/// The parent of a path, or null at (or above) the given root.
export function parentPath(path: string, root = "/"): string | null {
  if (path === root || path === "/") return null;
  const i = path.lastIndexOf("/");
  if (i <= 0) return path.startsWith("/") && root === "/" ? "/" : null;
  const parent = path.slice(0, i);
  if (!parent.startsWith(root)) return null;
  return parent;
}
