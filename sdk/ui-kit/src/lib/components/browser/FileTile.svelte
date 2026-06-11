<script lang="ts">
  /// One grid tile: a larger icon over a two-line name. Same content
  /// voice as the row (13px name, secondary dim for hidden entries),
  /// same selection states.
  import type { Snippet } from "svelte";
  import type { FileEntry } from "./types";
  import { entryIcon } from "./icons";

  let {
    entry,
    selected = false,
    focused = false,
    icon,
    ontileclick,
    ontiledblclick,
    ontilecontextmenu,
  }: {
    entry: FileEntry;
    selected?: boolean;
    focused?: boolean;
    icon?: Snippet<[FileEntry]>;
    ontileclick?: (e: MouseEvent) => void;
    ontiledblclick?: (e: MouseEvent) => void;
    ontilecontextmenu?: (e: MouseEvent) => void;
  } = $props();

  const Icon = $derived(entryIcon(entry));
</script>

<div
  class="file-tile"
  class:selected
  class:focused
  class:hidden-entry={entry.is_hidden}
  role="gridcell"
  tabindex={-1}
  aria-selected={selected}
  onclick={ontileclick}
  ondblclick={ontiledblclick}
  oncontextmenu={ontilecontextmenu}
>
  <span class="ft-icon">
    {#if icon}
      {@render icon(entry)}
    {:else}
      <Icon size={32} strokeWidth={1.25} />
    {/if}
  </span>
  <span class="ft-name">{entry.name}</span>
</div>

<style>
  .file-tile {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 12px 8px 8px;
    border-radius: var(--radius-input);
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .file-tile:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .file-tile.selected {
    background: color-mix(in srgb, var(--color-accent, var(--primary)) 15%, transparent);
  }
  .file-tile.focused {
    outline: 1px solid color-mix(in srgb, var(--color-accent, var(--primary)) 45%, transparent);
    outline-offset: -1px;
  }

  .ft-icon {
    display: inline-flex;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .ft-name {
    font-size: 0.8125rem;
    color: var(--foreground);
    text-align: center;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    max-width: 100%;
  }
  .hidden-entry .ft-name {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
