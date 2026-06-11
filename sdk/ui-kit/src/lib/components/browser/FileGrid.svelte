<script lang="ts">
  /// The icon view: responsive tile grid over the same entries and
  /// the same event/selection contract as the list. The browser's
  /// keyboard grammar passes a column stride so arrows move
  /// two-dimensionally.
  import type { Snippet } from "svelte";
  import type { FileEntry } from "./types";
  import FileTile from "./FileTile.svelte";

  let {
    entries,
    selectedIndices,
    cursorIndex = null,
    icon,
    onrowevent,
  }: {
    entries: FileEntry[];
    selectedIndices: ReadonlySet<number>;
    cursorIndex?: number | null;
    icon?: Snippet<[FileEntry]>;
    onrowevent?: (
      kind: "click" | "dblclick" | "contextmenu",
      index: number,
      e: MouseEvent,
    ) => void;
  } = $props();
</script>

<div class="file-grid" role="grid" aria-label="Files">
  {#each entries as entry, i (entry.name)}
    <FileTile
      {entry}
      {icon}
      selected={selectedIndices.has(i)}
      focused={cursorIndex === i}
      ontileclick={(e) => onrowevent?.("click", i, e)}
      ontiledblclick={(e) => onrowevent?.("dblclick", i, e)}
      ontilecontextmenu={(e) => onrowevent?.("contextmenu", i, e)}
    />
  {/each}
</div>

<style>
  .file-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(7rem, 1fr));
    gap: 4px;
    padding: 8px;
    align-content: start;
  }
</style>
