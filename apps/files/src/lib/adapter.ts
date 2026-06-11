/// The FM app's browser adapter: the one seam between the shared kit
/// browser and this host's Tauri commands. The FM is the unconfined
/// surface, so paths are absolute and the root is `/`.

import { invoke } from "@tauri-apps/api/core";
import type {
  BrowserAdapter,
  FileEntry,
  SortSpec,
} from "@arlen/ui-kit/components/browser";

export const fmAdapter: BrowserAdapter = {
  list: (path: string, sort: SortSpec) =>
    invoke<FileEntry[]>("files_list", {
      path,
      sort: sort.key,
      foldersFirst: sort.foldersFirst,
      ascending: sort.ascending,
    }),
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
