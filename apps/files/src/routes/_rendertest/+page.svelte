<script lang="ts">
  /// Headless render harness for FmInfoPanel. UI-AFFORDANCE verification ONLY,
  /// NOT a behaviour claim. Mocks the daemon over Tauri IPC (only when no Tauri
  /// runtime is present, so it can never hijack the real app) and renders the
  /// real FmInfoPanel across its states: a file with related projects + the
  /// as-of view, an image with EXIF + permissions, a folder, a symlink. The real
  /// permission writes + KG behaviour are proven by the coder's seed + tests, not
  /// this mock. Not shipped in any nav; a dev/test route only.
  import { onMount } from "svelte";
  import { tauriAvailable } from "$lib/tauri";
  import FmInfoPanel from "$lib/components/FmInfoPanel.svelte";
  import type { FileEntry } from "@arlen/ui-kit/components/browser";

  const FILE = "/demo/thesis-draft.md";
  const IMAGE = "/demo/inn-sunset.jpg";
  const FOLDER = "/demo/Projects";
  const SYMLINK = "/demo/shortcut";

  const liveRel = [
    { label: "Part of", target: "Thesis writeup", target_id: "p-thesis" },
    { label: "Part of", target: "Reading list", target_id: "p-reading" },
  ];
  const pastRel = [{ label: "Part of", target: "Proposal draft", target_id: "p-proposal" }];

  const MODIFIED = 1782300000;
  const CREATED = 1781000000;
  const base = (over: Record<string, unknown>) => ({
    conventional: {
      kind: "file",
      size: 48213,
      mode: 0o644,
      modified_unix: MODIFIED,
      created_unix: CREATED,
      ...over,
    },
    woher: [
      { label: "Accessed by", detail: "Files" },
      { label: "Also accessed by", detail: "another app" },
    ],
    verwandt: [] as typeof liveRel,
    zugriff: { readable_by: [] as string[], manage_link: "settings:ai" },
  });

  const fileInfo = { ...base({}), verwandt: liveRel };
  const imageInfo = base({ kind: "file", size: 2_517_000 });
  const folderInfo = { ...base({ kind: "directory", size: 0, mode: 0o755 }), woher: [] };
  const symlinkInfo = { ...base({ kind: "symlink", mode: 0o777 }), woher: [] };

  let ready = $state(false);
  onMount(async () => {
    if (!tauriAvailable) {
      const { mockIPC } = await import("@tauri-apps/api/mocks");
      mockIPC((cmd, args) => {
        const a = (args ?? {}) as Record<string, unknown>;
        if (cmd === "files_info") {
          if (a.path === IMAGE) return imageInfo;
          if (a.path === FOLDER) return folderInfo;
          if (a.path === SYMLINK) return symlinkInfo;
          return fileInfo;
        }
        if (cmd === "files_verwandt_as_of") return a.path === FILE ? pastRel : [];
        if (cmd === "files_get_exif_tags")
          return { description: "Sunset over the Inn", artist: "Tim", copyright: null };
        if (cmd === "files_set_permissions" || cmd === "files_set_exif_tags") return null;
        if (cmd === "files_thumbnail") return null;
        throw new Error(`unmocked: ${cmd}`);
      });
    }
    ready = true;
  });

  const entry = (name: string, kind: string): FileEntry =>
    ({
      name,
      is_hidden: false,
      kind,
      size: 0,
      modified_unix: MODIFIED,
      readonly: false,
      symlink_target: null,
      full_path: null,
      restore_token: null,
    }) as unknown as FileEntry;
</script>

<div class="harness">
  {#if ready}
    <div class="host">
      <h2>File: related + as-of</h2>
      <FmInfoPanel path={FILE} entry={entry("thesis-draft.md", "file")} onnavigate={() => {}} />
    </div>
    <div class="host">
      <h2>Image: EXIF + permissions</h2>
      <FmInfoPanel path={IMAGE} entry={entry("inn-sunset.jpg", "file")} onnavigate={() => {}} />
    </div>
    <div class="host">
      <h2>Folder</h2>
      <FmInfoPanel path={FOLDER} entry={entry("Projects", "directory")} onnavigate={() => {}} />
    </div>
    <div class="host">
      <h2>Symlink</h2>
      <FmInfoPanel path={SYMLINK} entry={entry("shortcut", "symlink")} onnavigate={() => {}} />
    </div>
  {/if}
</div>

<style>
  .harness {
    display: flex;
    gap: 20px;
    padding: 24px;
    min-height: 100vh;
    background: var(--background);
  }
  .host {
    display: flex;
    flex-direction: column;
  }
  h2 {
    margin: 0 0 8px;
    font-size: 0.75rem;
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
