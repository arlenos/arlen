<script lang="ts">
  /// One listing row in the detail view: icon, name, size, modified.
  /// Dumb by design — selection, cursor and events come from the
  /// list. Hidden entries speak at the secondary dim; a symlink shows
  /// its target as quiet trailing text. The `icon` snippet is the
  /// seam for themed and KG-state icons later.
  import type { Snippet } from "svelte";
  import type { FileEntry } from "./types";
  import { entryIcon } from "./icons";
  import { formatModified, formatSize } from "./format";

  let {
    entry,
    selected = false,
    focused = false,
    now,
    icon,
    onrowclick,
    onrowdblclick,
    onrowcontextmenu,
  }: {
    entry: FileEntry;
    selected?: boolean;
    /// The keyboard cursor sits here.
    focused?: boolean;
    /// Injectable clock for stable screenshots.
    now?: number;
    icon?: Snippet<[FileEntry]>;
    onrowclick?: (e: MouseEvent) => void;
    onrowdblclick?: (e: MouseEvent) => void;
    onrowcontextmenu?: (e: MouseEvent) => void;
  } = $props();

  const Icon = $derived(entryIcon(entry));
</script>

<div
  class="file-row"
  class:selected
  class:focused
  class:hidden-entry={entry.is_hidden}
  role="row"
  aria-selected={selected}
  onclick={onrowclick}
  ondblclick={onrowdblclick}
  oncontextmenu={onrowcontextmenu}
>
  <span class="fr-main" role="gridcell">
    <span class="fr-icon">
      {#if icon}
        {@render icon(entry)}
      {:else}
        <Icon size={16} strokeWidth={1.75} />
      {/if}
    </span>
    <span class="fr-name">{entry.name}</span>
    {#if entry.symlink_target}
      <span class="fr-target">{entry.symlink_target}</span>
    {/if}
  </span>
  <span class="fr-size" role="gridcell">{formatSize(entry.size)}</span>
  <span class="fr-modified" role="gridcell">
    {formatModified(entry.modified_unix, now)}
  </span>
</div>

<style>
  .file-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 6rem 9rem;
    align-items: center;
    gap: 8px;
    height: 2rem;
    padding: 0 8px;
    border-radius: var(--radius-input);
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .file-row:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .file-row.selected {
    background: color-mix(in srgb, var(--color-accent, var(--primary)) 15%, transparent);
  }
  .file-row.focused {
    outline: 1px solid color-mix(in srgb, var(--color-accent, var(--primary)) 45%, transparent);
    outline-offset: -1px;
  }

  .fr-main {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .fr-icon {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .fr-name {
    font-size: 0.8125rem;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hidden-entry .fr-name {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .fr-target {
    flex-shrink: 1;
    min-width: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .fr-size {
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    text-align: right;
  }
  .fr-modified {
    font-size: 0.75rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
