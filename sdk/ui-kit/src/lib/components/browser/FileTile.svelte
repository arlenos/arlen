<script lang="ts">
  /// One grid tile: a fixed media box over a two-line name. The box
  /// shows the thumbnail when the host resolved one (cover-cropped,
  /// never stretched) and the icon otherwise — including while the
  /// image loads, so the swap is opacity-only and the layout never
  /// shifts. Fixed box + fixed name height make the tile height a
  /// constant the grid windowing can rely on. Same content voice as
  /// the row (13px name, secondary dim for hidden entries).
  import type { Snippet } from "svelte";
  import type { FileEntry } from "./types";
  import { entryIcon } from "./icons";

  let {
    entry,
    selected = false,
    focused = false,
    badge = null,
    thumbnail = null,
    icon,
    ontileclick,
    ontiledblclick,
    ontilecontextmenu,
  }: {
    entry: FileEntry;
    selected?: boolean;
    focused?: boolean;
    /// The quiet bottom-right corner signal: the filetype on a
    /// thumbnail (an image hides what it is), later the KG-state
    /// overlay. Rendered only over a loaded thumbnail — the corner
    /// stays empty on plain icons by design (the icon already says
    /// the type).
    badge?: string | null;
    /// A renderable thumbnail URL, or null for the icon.
    thumbnail?: string | null;
    icon?: Snippet<[FileEntry]>;
    ontileclick?: (e: MouseEvent) => void;
    ontiledblclick?: (e: MouseEvent) => void;
    ontilecontextmenu?: (e: MouseEvent) => void;
  } = $props();

  const Icon = $derived(entryIcon(entry));

  // Zero-width break opportunities after _/- and before dots: a
  // two-line name breaks at a separator ("IMG_0003" / ".webp"), not
  // mid-extension ("…web" / "p"); overflow-wrap stays the last
  // resort for separator-less names.
  const wrapName = $derived(
    entry.name.replace(/([_-])/g, "$1\u200b").replace(/\./g, "\u200b."),
  );

  let loaded = $state(false);
  let failed = $state(false);
  $effect(() => {
    void thumbnail;
    loaded = false;
    failed = false;
  });
  const showImage = $derived(thumbnail !== null && !failed);
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
  <span class="ft-media">
    {#if showImage}
      <img
        class="ft-thumb"
        class:ready={loaded}
        src={thumbnail}
        alt=""
        draggable="false"
        onload={() => (loaded = true)}
        onerror={() => (failed = true)}
      />
    {/if}
    {#if !showImage || !loaded}
      {#if icon}
        {@render icon(entry)}
      {:else}
        <Icon size={32} strokeWidth={1.25} />
      {/if}
    {/if}
    {#if badge && showImage && loaded}
      <span class="ft-badge">{badge}</span>
    {/if}
  </span>
  <span class="ft-name">{wrapName}</span>
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

  /* The fixed media box: full tile width, constant height. Icon and
     image share it, so the loading swap never moves the layout. */
  .ft-media {
    position: relative;
    width: 100%;
    height: var(--browser-thumb-box, 72px);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ft-thumb {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: calc(var(--radius-input) - 2px);
    opacity: 0;
    transition: opacity var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .ft-thumb.ready {
    opacity: 1;
  }
  /* Over a photo the badge needs its own contrast: a dark scrim and
     light text, independent of the theme's foreground. */
  .ft-badge {
    position: absolute;
    right: 3px;
    bottom: 3px;
    padding: 0 4px;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, black 55%, transparent);
    color: color-mix(in srgb, white 92%, transparent);
    font-size: 0.625rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.02em;
    line-height: 1.5;
  }

  /* A fixed two-line box (not clamp-only): one-line names get the
     same tile height, which the grid windowing depends on. */
  .ft-name {
    font-size: 0.8125rem;
    line-height: 16px;
    height: 32px;
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
