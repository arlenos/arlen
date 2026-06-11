<script lang="ts">
  /// The on-demand info panel (KG quiet place #2): conventional
  /// Get-Info on top, then the graph sections — Where from, Related,
  /// Access — rendered only when the graph has something to say. The
  /// access view is read-only with one deep link; capabilities are
  /// managed in Settings, never here.
  import { writable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { X } from "lucide-svelte";
  import {
    entryIcon,
    formatModified,
    formatSize,
    type FileEntry,
  } from "@arlen/ui-kit/components/browser";
  import { openPath } from "$lib/adapter";

  let {
    path,
    entry,
    onclose,
  }: {
    /// The full path of the inspected entry.
    path: string;
    /// Its listing entry (for icon and name; null while unknown).
    entry: FileEntry | null;
    onclose?: () => void;
  } = $props();

  interface Info {
    conventional: {
      kind: string;
      size: number;
      mode: number;
      modified_unix: number;
    };
    woher: { label: string; detail: string }[];
    verwandt: { label: string; target: string }[];
    zugriff: { readable_by: string[]; manage_link: string };
  }

  const info = writable<Info | null>(null);

  $effect(() => {
    const p = path;
    invoke<Info>("files_info", { path: p })
      .then((i) => info.set(i))
      .catch(() => info.set(null));
  });

  const name = $derived(path.split("/").filter(Boolean).pop() ?? "/");
  const Icon = $derived(entry ? entryIcon(entry) : null);

  const kindLabel = (kind: string): string =>
    kind === "directory" ? "Folder" : kind === "symlink" ? "Link" : "File";
</script>

<aside class="info-panel" aria-label="Info">
  <div class="ip-head">
    <span class="ip-name">{name}</span>
    <button class="ip-close" aria-label="Close info" onclick={() => onclose?.()}>
      <X size={14} strokeWidth={2} />
    </button>
  </div>

  <div class="ip-preview">
    {#if Icon}
      <Icon size={48} strokeWidth={1} />
    {/if}
  </div>

  {#if $info}
    <div class="ip-facts">
      <span>{kindLabel($info.conventional.kind)}</span>
      {#if $info.conventional.kind !== "directory"}
        <span>{formatSize($info.conventional.size)}</span>
      {/if}
      <span>changed {formatModified($info.conventional.modified_unix)}</span>
    </div>

    {#if $info.woher.length > 0}
      <div class="ip-section">
        <span class="ip-label">Where from</span>
        {#each $info.woher as line (line.label + line.detail)}
          <div class="ip-row">
            <span class="ip-key">{line.label}</span>
            <span class="ip-value">{line.detail}</span>
          </div>
        {/each}
      </div>
    {/if}

    {#if $info.verwandt.length > 0}
      <div class="ip-section">
        <span class="ip-label">Related</span>
        {#each $info.verwandt as line (line.label + line.target)}
          <div class="ip-row">
            <span class="ip-key">{line.label}</span>
            <span class="ip-value">{line.target}</span>
          </div>
        {/each}
      </div>
    {/if}

    {#if $info.zugriff.readable_by.length > 0}
      <div class="ip-section">
        <span class="ip-label">Access</span>
        <div class="ip-row">
          <span class="ip-key">Readable by</span>
          <span class="ip-value">{$info.zugriff.readable_by.join(", ")}</span>
        </div>
        <button
          class="ip-manage"
          onclick={() => void openPath($info.zugriff.manage_link)}
        >
          Manage access in Settings
        </button>
      </div>
    {/if}
  {/if}
</aside>

<style>
  .info-panel {
    width: 17rem;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px;
    border-left: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    overflow-y: auto;
  }

  .ip-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ip-name {
    flex: 1;
    min-width: 0;
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ip-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ip-close:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }

  .ip-preview {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 7rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }

  .ip-facts {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 12px;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .ip-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ip-label {
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ip-row {
    display: flex;
    gap: 8px;
    font-size: 0.75rem;
  }
  .ip-key {
    width: 6.5rem;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ip-value {
    flex: 1;
    min-width: 0;
    color: var(--foreground);
    overflow-wrap: anywhere;
  }

  .ip-manage {
    align-self: flex-start;
    margin-top: 4px;
    height: var(--height-control, 28px);
    padding: 0 12px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-input);
    background: var(--control-bg);
    color: var(--foreground);
    font-size: 0.75rem;
    font-weight: 500;
    transition: background-color var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .ip-manage:hover {
    background: var(--control-bg-hover);
  }
</style>
