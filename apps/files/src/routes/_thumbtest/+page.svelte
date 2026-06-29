<script lang="ts">
  /// Headless render harness for list-view thumbnails. UI-AFFORDANCE
  /// verification ONLY. A mock adapter returns a small listing (image files +
  /// non-images + a folder) and a sample thumbnail data-URI for the images, so
  /// the real FileBrowser shows the new small row previews in the LIST view
  /// (icons for the rest) and the already-wired tile previews in the GRID view
  /// (`?view=grid`). Proves the kit plumbing; the real thumbnails are the
  /// daemon's `files_thumbnail`. Dev route only.
  import { onMount } from "svelte";
  import {
    FileBrowser,
    createBrowserState,
    DEFAULT_COLUMNS,
    type BrowserAdapter,
    type FileEntry,
  } from "@arlen/ui-kit/components/browser";

  const thumb = (h: number) =>
    `data:image/svg+xml;base64,${btoa(
      `<svg xmlns='http://www.w3.org/2000/svg' width='80' height='80'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='hsl(${h},55%,60%)'/><stop offset='1' stop-color='hsl(${(h + 60) % 360},55%,38%)'/></linearGradient></defs><rect width='80' height='80' fill='url(#g)'/><circle cx='52' cy='30' r='12' fill='hsl(${h},70%,82%)'/></svg>`,
    )}`;

  const entry = (name: string, kind: string, size: number): FileEntry =>
    ({
      name,
      is_hidden: false,
      kind,
      size,
      modified_unix: 1_786_000_000,
      readonly: false,
      symlink_target: null,
      full_path: `/demo/${name}`,
      restore_token: null,
    }) as unknown as FileEntry;

  const ENTRIES: FileEntry[] = [
    entry("Photos", "directory", 0),
    entry("inn-sunset.jpg", "file", 2_517_000),
    entry("berg.png", "file", 1_240_000),
    entry("diagram.webp", "file", 88_000),
    entry("notes.md", "file", 12_400),
    entry("budget.ods", "file", 41_000),
    entry("archive.zip", "file", 9_900_000),
    entry("cover.gif", "file", 320_000),
  ];

  const IMG = /\.(png|jpe?g|gif|bmp|webp)$/i;
  const adapter: BrowserAdapter = {
    list: async () => ENTRIES,
    thumbnail: async (_path, e) =>
      e.kind === "file" && IMG.test(e.name) ? thumb((e.name.length * 47) % 360) : null,
  };

  let controller = $state<ReturnType<typeof createBrowserState> | null>(null);
  onMount(() => {
    const c = createBrowserState(adapter, { initial: "/demo", allowVirtual: false });
    if (new URLSearchParams(window.location.search).get("view") === "grid") {
      c.viewMode.set("grid");
    }
    controller = c;
  });
</script>

<div class="harness">
  {#if controller}
    <FileBrowser {controller} columns={DEFAULT_COLUMNS} emptyLabel="Empty" />
  {/if}
</div>

<style>
  .harness {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--background);
    color: var(--foreground);
  }
</style>
