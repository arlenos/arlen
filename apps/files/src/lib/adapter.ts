/// The FM app's browser adapter: the one seam between the shared kit
/// browser and this host's Tauri commands. The FM is the unconfined
/// surface, so paths are absolute and the root is `/`.

import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";
import type {
  BrowserAdapter,
  FileEntry,
  SortSpec,
} from "@arlen/ui-kit/components/browser";
import { isVirtualLocation } from "@arlen/ui-kit/components/browser";
import { viewAsOfChoice, choiceToMicros } from "./asof";
import {
  archiveListing,
  sortEntries,
  splitArchivePath,
  type ArchiveEntry,
} from "./archive";

/// What the sandboxed decoder can actually thumbnail; mirrors
/// `is_thumbnailable` in src-tauri/src/thumbnail.rs — keep the two in
/// step. svg/ico/avif/tiff get the image icon but no thumbnail (the
/// worker cannot decode them).
const THUMBNAILABLE = /\.(png|jpe?g|gif|bmp|webp)$/i;

export const fmAdapter: BrowserAdapter = {
  list: async (path: string, sort: SortSpec) => {
    // Diagnostic for the virtual-location navigation bug (Trash/Recent showed home
    // on metal): logging the path the listing ran for + whether it classified as
    // virtual localises the chain - it confirms navigate->load reached list with
    // the virtual key and that it routes to files_list_location, paired with the
    // backend log on that command.
    void invoke("frontend_log", {
      level: "info",
      msg: `fmAdapter.list: path=${path} virtual=${isVirtualLocation(path)}`,
    });
    // Browse an archive as a folder (FM-R12): a path at or inside an archive
    // lists the archive's contents (read-only) instead of the real filesystem.
    const archive = splitArchivePath(path);
    if (archive) {
      const entries = await invoke<ArchiveEntry[]>("files_archive_list", {
        archive: archive.archive,
      });
      return sortEntries(archiveListing(entries, archive.inner), sort);
    }
    // A virtual KG location (recent / trash / project:<id> / search:<query>):
    // the backend returns its members scattered across folders (each with its
    // own full_path), unsorted, so we sort client-side like the archive path.
    if (isVirtualLocation(path)) {
      // When the whole-listing time-travel is on, read the location as of the
      // chosen past instant (only project membership is bitemporal; other
      // locations return their live members regardless).
      const asOf = choiceToMicros(get(viewAsOfChoice));
      const entries =
        asOf === null
          ? await invoke<FileEntry[]>("files_list_location", { location: path })
          : await invoke<FileEntry[]>("files_list_location_as_of", {
              location: path,
              asOfMicros: asOf,
            });
      return sortEntries(entries, sort);
    }
    return invoke<FileEntry[]>("files_list", {
      path,
      sort: sort.key,
      foldersFirst: sort.foldersFirst,
      ascending: sort.ascending,
    });
  },
  // Plain files only: a symlinked image would key the kit cache on
  // the link's entry while the bytes follow the target's mtime, so
  // links keep their icon.
  thumbnail: (path: string, entry: FileEntry) =>
    entry.kind === "file" && THUMBNAILABLE.test(entry.name)
      ? invoke<string | null>("files_thumbnail", { path })
      : Promise.resolve(null),
};

/// Open a non-directory entry with the system handler.
export async function openPath(path: string): Promise<void> {
  try {
    await invoke("files_open", { path });
  } catch {
    // The opener is honest about failure elsewhere (status line later);
    // an unopenable file must not crash the browser.
  }
}
