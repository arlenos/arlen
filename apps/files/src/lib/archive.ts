/// Browse an archive (.zip / .tar / .tar.gz / .tgz) as if it were a folder
/// (FM-R12). The backend `files_archive_list` returns the archive's full flat
/// listing; these pure helpers let the adapter detect an archive path, project
/// the flat listing into the folder-like rows directly under a sub-path, and
/// sort them like a real directory. Read-only - the archive is never mutated.

import type { FileEntry, SortSpec } from "@arlen/ui-kit/components/browser";

/// One entry of the backend `files_archive_list` reply (`archive::ArchiveEntry`).
export interface ArchiveEntry {
  /// The entry's path within the archive (a directory keeps its trailing `/`).
  path: string;
  size: number;
  is_dir: boolean;
}

/// The extensions `archive::is_extractable` recognises, mirrored here so the FM
/// decides to browse-into vs open-with without a round-trip.
const ARCHIVE_EXTS = [".zip", ".tar.gz", ".tgz", ".tar"];

/// Whether `name` is an archive the FM browses as a folder.
export function isArchiveName(name: string): boolean {
  const lower = name.toLowerCase();
  return ARCHIVE_EXTS.some((ext) => lower.endsWith(ext));
}

/// Split `path` at the first component that is an archive, so a path pointing at
/// or into an archive is browsable. Returns `{ archive, inner }` - `archive` is
/// the path up to and including the archive file, `inner` the (possibly empty)
/// path within it - or null when no component is an archive.
export function splitArchivePath(
  path: string,
): { archive: string; inner: string } | null {
  const parts = path.split("/");
  for (let i = 0; i < parts.length; i++) {
    if (parts[i] && isArchiveName(parts[i])) {
      return {
        archive: parts.slice(0, i + 1).join("/"),
        inner: parts.slice(i + 1).filter(Boolean).join("/"),
      };
    }
  }
  return null;
}

function makeEntry(name: string, isDir: boolean, size: number | null): FileEntry {
  return {
    name,
    kind: isDir ? "directory" : "file",
    size: isDir ? null : size,
    modified_unix: null,
    is_hidden: name.startsWith("."),
    readonly: true,
    symlink_target: null,
  };
}

/// Project the archive's flat `entries` into the `FileEntry` rows directly under
/// `inner` (the folder-like view): an entry exactly one level below `inner` is
/// itself; a deeper entry contributes its first intermediate component as a
/// synthesized directory (deduped against an explicit directory entry). Unsorted
/// - the caller sorts with [`sortEntries`].
export function archiveListing(entries: ArchiveEntry[], inner: string): FileEntry[] {
  const prefix = inner ? `${inner.replace(/\/+$/, "")}/` : "";
  const out: FileEntry[] = [];
  const directChildren = new Set<string>();
  const synthDirs = new Set<string>();

  for (const e of entries) {
    const p = e.path.replace(/\/+$/, "");
    if (prefix) {
      if (!p.startsWith(prefix)) continue;
    }
    const rest = p.slice(prefix.length);
    if (!rest) continue;
    const slash = rest.indexOf("/");
    if (slash === -1) {
      if (directChildren.has(rest)) continue;
      directChildren.add(rest);
      out.push(makeEntry(rest, e.is_dir, e.size));
    } else {
      synthDirs.add(rest.slice(0, slash));
    }
  }
  for (const dir of synthDirs) {
    if (!directChildren.has(dir)) out.push(makeEntry(dir, true, null));
  }
  return out;
}

/// Order a listing like the directory browser: folders first (when the spec
/// asks), then by the chosen key, honouring `ascending`. Matches what the
/// backend sort gives a real directory, so an archive view sorts consistently.
export function sortEntries(entries: FileEntry[], sort: SortSpec): FileEntry[] {
  const rank = (e: FileEntry) => (e.kind === "directory" ? 0 : 1);
  const ext = (name: string) => {
    const i = name.lastIndexOf(".");
    return i > 0 ? name.slice(i + 1).toLowerCase() : "";
  };
  const byKey = (a: FileEntry, b: FileEntry): number => {
    switch (sort.key) {
      case "size":
        return (a.size ?? 0) - (b.size ?? 0);
      case "modified":
        return (a.modified_unix ?? 0) - (b.modified_unix ?? 0);
      case "type":
        return ext(a.name).localeCompare(ext(b.name)) || a.name.localeCompare(b.name);
      default:
        return a.name.localeCompare(b.name, undefined, { numeric: true });
    }
  };
  return [...entries].sort((a, b) => {
    if (sort.foldersFirst && rank(a) !== rank(b)) return rank(a) - rank(b);
    const r = byKey(a, b);
    return sort.ascending ? r : -r;
  });
}
