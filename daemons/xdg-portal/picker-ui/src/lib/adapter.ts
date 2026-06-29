/// The picker's browser adapter: the one seam between the shared kit
/// browser and the confined picker's Tauri commands. The picker is the
/// confined surface - it lists only through `list_directory` (the
/// daemon's cap-std-scoped FS) and never reaches a virtual KG location.
///
/// `list_directory` returns the lean `DirEntry` (name / path /
/// isDirectory / isHidden) with no size or mtime, so those kit columns
/// render blank until the daemon enriches the listing (the
/// `list_directory` enrichment seam, flagged in arlen-ui-reports.md).
/// The picker sorts client-side because the daemon command does not.

import { invoke } from "@tauri-apps/api/core";
import type {
  BrowserAdapter,
  EntryKind,
  FileEntry,
  SortSpec,
} from "@arlen/ui-kit/components/browser";
import type { DirEntry } from "$lib/types/protocol";

/// What the sandboxed picker-side decoder can thumbnail; mirrors the
/// FM's THUMBNAILABLE set (apps/files/src/lib/adapter.ts). svg/ico/avif
/// keep the image icon but get no thumbnail.
const THUMBNAILABLE = /\.(png|jpe?g|gif|bmp|webp)$/i;

function toEntry(d: DirEntry): FileEntry {
  const kind: EntryKind = d.isDirectory ? "directory" : "file";
  return {
    name: d.name,
    kind,
    // The daemon's DirEntry carries no size/mtime; the kit renders the
    // blank cell honestly rather than a fabricated value.
    size: null,
    modified_unix: null,
    is_hidden: d.isHidden,
    readonly: false,
    symlink_target: null,
  };
}

function ext(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

/// Sort a listing per the controller's SortSpec. Folders-first always
/// (the controller asks for it); within a group, by the active key.
/// Size and Modified collapse to the name order because the lean
/// DirEntry has neither - honest until the listing is enriched.
function sortEntries(entries: FileEntry[], sort: SortSpec): FileEntry[] {
  const dir = sort.ascending ? 1 : -1;
  return [...entries].sort((a, b) => {
    if (sort.foldersFirst && a.kind !== b.kind) {
      if (a.kind === "directory") return -1;
      if (b.kind === "directory") return 1;
    }
    let cmp = 0;
    if (sort.key === "type") {
      cmp = ext(a.name).localeCompare(ext(b.name));
    }
    if (cmp === 0) {
      cmp = a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
    }
    return cmp * dir;
  });
}

export const pickerAdapter: BrowserAdapter = {
  list: async (path: string, sort: SortSpec) => {
    const raw = await invoke<DirEntry[]>("list_directory", { path });
    return sortEntries(raw.map(toEntry), sort);
  },
  // Plain files with a decodable extension; the picker-side thumbnail
  // command is confined to the daemon's cap-std root. Any failure
  // (command absent during the frontend mock, or a decode error) falls
  // back to the icon - the tile never breaks.
  thumbnail: async (path: string, entry: FileEntry) => {
    if (entry.kind !== "file" || !THUMBNAILABLE.test(entry.name)) return null;
    try {
      return await invoke<string | null>("picker_thumbnail", { path });
    } catch {
      return null;
    }
  },
};
