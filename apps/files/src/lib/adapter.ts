/// The FM app's browser adapter: the one seam between the shared kit
/// browser and this host's Tauri commands. The FM is the unconfined
/// surface, so paths are absolute and the root is `/`.

import { invoke } from "@tauri-apps/api/core";
import type {
  BrowserAdapter,
  FileEntry,
  SortSpec,
} from "@arlen/ui-kit/components/browser";

/// What the sandboxed decoder can actually thumbnail; mirrors
/// `is_thumbnailable` in src-tauri/src/thumbnail.rs — keep the two in
/// step. svg/ico/avif/tiff get the image icon but no thumbnail (the
/// worker cannot decode them).
const THUMBNAILABLE = /\.(png|jpe?g|gif|bmp|webp)$/i;

export const fmAdapter: BrowserAdapter = {
  list: (path: string, sort: SortSpec) =>
    invoke<FileEntry[]>("files_list", {
      path,
      sort: sort.key,
      foldersFirst: sort.foldersFirst,
      ascending: sort.ascending,
    }),
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
