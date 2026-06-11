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

  import { tick } from "svelte";

  let {
    entry,
    selected = false,
    focused = false,
    renaming = false,
    now,
    icon,
    onrowclick,
    onrowdblclick,
    onrowcontextmenu,
    onrename,
  }: {
    entry: FileEntry;
    selected?: boolean;
    /// The keyboard cursor sits here.
    focused?: boolean;
    /// The name shows as an inline edit field.
    renaming?: boolean;
    /// Injectable clock for stable screenshots.
    now?: number;
    icon?: Snippet<[FileEntry]>;
    onrowclick?: (e: MouseEvent) => void;
    onrowdblclick?: (e: MouseEvent) => void;
    onrowcontextmenu?: (e: MouseEvent) => void;
    /// Inline rename finished (Enter or blur); Escape reports the
    /// old name, which callers treat as a no-op.
    onrename?: (newName: string) => void;
  } = $props();

  const Icon = $derived(entryIcon(entry));

  let draft = $state("");
  let inputRef = $state<HTMLInputElement | null>(null);
  // Enter commits and unmounts the input, which fires blur — the
  // flag keeps that from committing a second time.
  let committed = false;

  // Entering rename: prefill and select the stem (not the extension),
  // the way every file manager does.
  $effect(() => {
    if (!renaming) return;
    draft = entry.name;
    committed = false;
    tick().then(() => {
      if (!inputRef) return;
      inputRef.focus();
      const dot = entry.name.lastIndexOf(".");
      inputRef.setSelectionRange(0, dot > 0 ? dot : entry.name.length);
    });
  });

  function commit() {
    if (committed) return;
    committed = true;
    const name = draft.trim();
    onrename?.(name.length > 0 ? name : entry.name);
  }

  function onRenameKeydown(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      committed = true;
      onrename?.(entry.name);
    }
  }
</script>

<div
  class="file-row"
  class:selected
  class:focused
  class:hidden-entry={entry.is_hidden}
  role="row"
  tabindex={-1}
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
    {#if renaming}
      <input
        bind:this={inputRef}
        bind:value={draft}
        class="fr-rename"
        aria-label="New name"
        spellcheck="false"
        onkeydown={onRenameKeydown}
        onblur={commit}
        onclick={(e) => e.stopPropagation()}
        ondblclick={(e) => e.stopPropagation()}
      />
    {:else}
      <span class="fr-name">{entry.name}</span>
      {#if entry.symlink_target}
        <span class="fr-target">{entry.symlink_target}</span>
      {/if}
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
  .fr-rename {
    flex: 1;
    min-width: 0;
    height: var(--height-control-compact, 24px);
    padding: 0 6px;
    border: 1px solid var(--control-border-hover, var(--control-border));
    border-radius: var(--radius-chip);
    background: var(--color-bg-input, var(--background));
    color: var(--foreground);
    font-size: 0.8125rem;
    outline: none;
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
