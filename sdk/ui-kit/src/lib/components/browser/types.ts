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
}

/// A navigable place in the sidebar (label + icon key + path). The
/// icon key resolves through the kit icon map; `offline` renders the
/// gray status dot.
export interface Place {
  label: string;
  icon: string;
  path: string;
  offline?: boolean;
}

/// One sidebar group of places.
export interface PlaceGroup {
  label: string;
  places: Place[];
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
