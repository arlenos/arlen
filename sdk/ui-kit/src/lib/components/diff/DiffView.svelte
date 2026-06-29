<script lang="ts">
  /// The change-diff body: a list of changed files, each an expandable unified
  /// diff with +/- gutters and tinted rows (a muted green/red, the restrained
  /// register the rest of the system uses, not a vibrant web diff). A single
  /// small change opens expanded; multi-file or large changes open collapsed so
  /// a host card stays scannable. Pure + presentational: it renders the diff
  /// model, the host owns approve/undo.
  import { ChevronRight, FilePlus2, FileMinus2, FilePen, ArrowRight } from "@lucide/svelte";
  import type { DiffFile } from "./diff";

  let { files, collapsed = false }: { files: DiffFile[]; collapsed?: boolean } = $props();

  const STATUS_ICON = {
    added: FilePlus2,
    deleted: FileMinus2,
    renamed: ArrowRight,
    modified: FilePen,
  } as const;

  // A single small file opens expanded; anything larger or multi-file collapses,
  // so a big change does not flood the host.
  function lineCount(f: DiffFile): number {
    return f.hunks.reduce((n, h) => n + h.lines.length, 0);
  }
  const initial = (f: DiffFile): boolean =>
    !collapsed && files.length === 1 && lineCount(f) <= 28;

  // Only explicit toggles are stored; an untouched file falls back to its
  // size-based default, so this never has to be seeded from `files`.
  let overrides = $state<Record<string, boolean>>({});
  const isOpen = (f: DiffFile): boolean => overrides[f.path] ?? initial(f);
</script>

<div class="diff">
  {#each files as file (file.path)}
    {@const Icon = STATUS_ICON[file.status]}
    <div class="file">
      <button
        class="file-head"
        aria-expanded={isOpen(file)}
        onclick={() => (overrides[file.path] = !isOpen(file))}
      >
        <ChevronRight size={13} strokeWidth={2} class={`twist ${isOpen(file) ? "on" : ""}`} />
        <Icon size={13} strokeWidth={2} class={`status status-${file.status}`} />
        <span class="path">
          {#if file.status === "renamed" && file.oldPath}
            <span class="old">{file.oldPath}</span>
            <ArrowRight size={11} strokeWidth={2} />
          {/if}
          {file.path}
        </span>
        <span class="counts">
          {#if file.additions}<span class="add">+{file.additions}</span>{/if}
          {#if file.deletions}<span class="del">-{file.deletions}</span>{/if}
        </span>
      </button>

      {#if isOpen(file)}
        <div class="body">
          {#each file.hunks as hunk, hi (hi)}
            <div class="hunk-head">{hunk.header}</div>
            {#each hunk.lines as line, li (li)}
              <div class={`row ${line.kind}`}>
                <span class="gutter">{line.kind === "add" ? "+" : line.kind === "del" ? "-" : ""}</span>
                <span class="code">{line.text || " "}</span>
              </div>
            {/each}
          {/each}
        </div>
      {/if}
    </div>
  {/each}
</div>

<style>
  .diff {
    --diff-add: var(--color-success, #8fae74);
    --diff-del: var(--color-error, #c96a6a);
    display: flex;
    flex-direction: column;
    gap: 0.375rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    overflow: hidden;
    background: color-mix(in srgb, var(--foreground) 2%, transparent);
  }
  .file + .file {
    border-top: 1px solid var(--color-border);
  }
  .file-head {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    width: 100%;
    padding: 0.375rem 0.5rem;
    border: none;
    background: transparent;
    color: var(--foreground);
    font-size: 0.75rem;
    text-align: left;
  }
  .file-head:hover {
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
  }
  .file-head :global(.twist) {
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    transition: transform var(--duration-fast) var(--ease-out);
  }
  .file-head :global(.twist.on) {
    transform: rotate(90deg);
  }
  .file-head :global(.status) {
    flex-shrink: 0;
  }
  .file-head :global(.status-added) {
    color: var(--diff-add);
  }
  .file-head :global(.status-deleted) {
    color: var(--diff-del);
  }
  .file-head :global(.status-modified),
  .file-head :global(.status-renamed) {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono, monospace);
  }
  .path .old {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    text-decoration: line-through;
  }
  .counts {
    flex-shrink: 0;
    display: inline-flex;
    gap: 0.4rem;
    font-family: var(--font-mono, monospace);
    font-size: 0.6875rem;
    font-variant-numeric: tabular-nums;
  }
  .counts .add {
    color: var(--diff-add);
  }
  .counts .del {
    color: var(--diff-del);
  }
  .body {
    border-top: 1px solid var(--color-border);
    font-family: var(--font-mono, monospace);
    font-size: 0.75rem;
    line-height: 1.5;
    overflow-x: auto;
  }
  .hunk-head {
    padding: 0.1rem 0.5rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    white-space: pre;
  }
  .row {
    display: flex;
    white-space: pre;
  }
  .row .gutter {
    flex-shrink: 0;
    width: 1.1rem;
    padding-left: 0.3rem;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
    user-select: none;
  }
  .row .code {
    padding-right: 0.6rem;
    white-space: pre;
  }
  .row.add {
    background: color-mix(in srgb, var(--diff-add) 14%, transparent);
  }
  .row.add .gutter {
    color: var(--diff-add);
  }
  .row.del {
    background: color-mix(in srgb, var(--diff-del) 14%, transparent);
  }
  .row.del .gutter {
    color: var(--diff-del);
  }
</style>
