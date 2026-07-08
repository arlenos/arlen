<script lang="ts">
  /// Miller columns: every level from the root to the current folder
  /// as its own column (Finder/ranger style). Ancestor listings come
  /// from the controller's cache (one prefetch per level, then one
  /// call per navigation); columns never reach above the controller's
  /// root, which keeps the confined picker honest. A single click on
  /// a directory navigates; the rightmost column carries the full
  /// selection grammar through the shared event contract.
  import { writable } from "svelte/store";
  import type { BrowserState } from "./controller";
  import type { FileEntry } from "./types";
  import { breadcrumb } from "./breadcrumb";
  import { entryIcon } from "./icons";
  import { ChevronRight } from "@lucide/svelte";

  let {
    controller,
    selectedIndices,
    cursorIndex = null,
    onrowevent,
  }: {
    controller: BrowserState;
    selectedIndices: ReadonlySet<number>;
    cursorIndex?: number | null;
    onrowevent?: (
      kind: "click" | "dblclick" | "contextmenu",
      index: number,
      e: MouseEvent,
    ) => void;
  } = $props();

  const path = $derived(controller.path);
  const entries = $derived(controller.entries);
  const showHidden = $derived(controller.showHidden);

  interface Column {
    path: string;
    entries: FileEntry[];
    /// The child segment this column descends into (highlight).
    childName: string | null;
  }

  // Ancestor listings land in a writable (IPC continuations), guarded
  // by a generation so a stale prefetch never paints.
  const ancestors = writable<Column[]>([]);
  let generation = 0;

  $effect(() => {
    const p = $path;
    void $showHidden;
    const root = controller.root;
    const crumbs = breadcrumb(p).filter(
      (c) => c.path === root || c.path.startsWith(root === "/" ? "/" : root + "/"),
    );
    const parents = crumbs.slice(0, -1);
    const gen = ++generation;
    Promise.all(
      parents.map(async (crumb, i) => ({
        path: crumb.path,
        entries: (await controller.listCached(crumb.path)).filter(
          (e) => $showHidden || !e.is_hidden,
        ),
        childName: crumbs[i + 1]?.name ?? null,
      })),
    )
      .then((cols) => {
        if (gen === generation) ancestors.set(cols);
      })
      .catch(() => {
        if (gen === generation) ancestors.set([]);
      });
  });

  function descend(column: Column, entry: FileEntry) {
    if (entry.kind !== "directory") return;
    const base = column.path === "/" ? "" : column.path;
    void controller.navigate(`${base}/${entry.name}`);
  }

  // The current folder is the rightmost column; keep it in view when
  // the trail grows or the mode opens. A left fade says "more
  // columns this way" whenever ancestors sit off-screen.
  let scroller = $state<HTMLDivElement | null>(null);
  let scrolledLeft = $state(false);
  $effect(() => {
    void $ancestors;
    if (scroller) {
      scroller.scrollLeft = scroller.scrollWidth;
      scrolledLeft = scroller.scrollLeft > 4;
    }
  });
</script>

<div class="miller-wrap">
{#if scrolledLeft}
  <div class="mc-fade" aria-hidden="true"></div>
{/if}
<div
  class="miller"
  bind:this={scroller}
  role="grid"
  aria-label="Folder columns"
  onscroll={() => (scrolledLeft = (scroller?.scrollLeft ?? 0) > 4)}
>
  {#each $ancestors as column (column.path)}
    <div class="mc-column">
      {#each column.entries as entry (entry.name)}
        {@const Icon = entryIcon(entry)}
        <button
          class="mc-row"
          class:on-trail={entry.name === column.childName}
          onclick={() => descend(column, entry)}
        >
          <span class="mc-icon"><Icon size={14} strokeWidth={1.75} /></span>
          <span class="mc-name">{entry.name}</span>
          {#if entry.kind === "directory"}
            <span class="mc-chevron">
              <ChevronRight size={12} strokeWidth={2} />
            </span>
          {/if}
        </button>
      {/each}
      {#if column.entries.length === 0}
        <div class="mc-empty">Empty</div>
      {/if}
    </div>
  {/each}

  <!-- The current folder: the live column with the full grammar. -->
  <div class="mc-column mc-current">
    {#each $entries as entry, i (entry.name)}
      {@const Icon = entryIcon(entry)}
      <button
        class="mc-row"
        class:selected={selectedIndices.has(i)}
        class:focused={cursorIndex === i}
        onclick={(e) => onrowevent?.("click", i, e)}
        ondblclick={(e) => onrowevent?.("dblclick", i, e)}
        oncontextmenu={(e) => onrowevent?.("contextmenu", i, e)}
      >
        <span class="mc-icon"><Icon size={14} strokeWidth={1.75} /></span>
        <span class="mc-name" class:dim={entry.is_hidden}>{entry.name}</span>
        {#if entry.kind === "directory"}
          <span class="mc-chevron">
            <ChevronRight size={12} strokeWidth={2} />
          </span>
        {/if}
      </button>
    {/each}
    {#if $entries.length === 0}
      <div class="mc-empty">Empty</div>
    {/if}
  </div>
</div>
</div>

<style>
  .miller-wrap {
    position: relative;
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .mc-fade {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    width: 16px;
    z-index: 1;
    pointer-events: none;
    background: linear-gradient(
      to right,
      var(--background),
      transparent
    );
  }

  .miller {
    display: flex;
    flex: 1;
    min-height: 0;
    overflow-x: auto;
    /* Resting positions align to column edges, so a partial column
       never lingers at the viewport edge. */
    scroll-snap-type: x proximity;
  }

  .mc-column {
    scroll-snap-align: start;
    width: 13rem;
    flex-shrink: 0;
    overflow-y: auto;
    padding: 4px;
    border-inline-end: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .mc-current {
    width: 16rem;
  }

  .mc-row {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: var(--height-control, 28px);
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    text-align: start;
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .mc-row:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .mc-row.on-trail {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .mc-row.selected {
    background: color-mix(in srgb, var(--color-accent, var(--primary)) 15%, transparent);
  }
  .mc-row.focused {
    outline: 1px solid color-mix(in srgb, var(--color-accent, var(--primary)) 45%, transparent);
    outline-offset: -1px;
  }

  .mc-icon {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .mc-name {
    flex: 1;
    min-width: 0;
    font-size: 0.8125rem;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mc-name.dim {
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .mc-chevron {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }

  .mc-empty {
    padding: 8px;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
