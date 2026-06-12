<script lang="ts">
  /// The path bar: clickable crumbs, and an editable path field
  /// behind Ctrl+L (the host toggles `editing`). A `homePath` prop
  /// collapses the home prefix into one "Home" crumb so deep paths
  /// read the way the places sidebar speaks. When the row runs out
  /// of room, the middle crumbs fold into one quiet ellipsis (first
  /// crumb and the last two stay legible) instead of every crumb
  /// squeezing toward one letter.
  import { tick } from "svelte";
  import { ChevronRight } from "@lucide/svelte";
  import { breadcrumb } from "./breadcrumb";
  import type { Crumb } from "./types";

  let {
    path,
    homePath,
    editing = $bindable(false),
    onnavigate,
  }: {
    path: string;
    /// Collapse this prefix into a single "Home" crumb.
    homePath?: string;
    /// True while the editable field shows (Ctrl+L); bindable.
    editing?: boolean;
    onnavigate?: (path: string) => void;
  } = $props();

  const crumbs = $derived.by(() => {
    const all = breadcrumb(path);
    if (homePath && (path === homePath || path.startsWith(homePath + "/"))) {
      const homeCrumbs = breadcrumb(homePath);
      return [
        { name: "Home", path: homePath } as Crumb,
        ...all.slice(homeCrumbs.length),
      ];
    }
    return all;
  });

  // Overflow handling: fold crumbs after the first into "…" one at a
  // time until the row fits. The crumbs keep their natural width
  // (no flex squeeze), so scrollWidth honestly reports the need.
  let navEl = $state<HTMLElement | null>(null);
  let folded = $state(0);
  // The deepest fold keeps the first crumb, "…" and the current one.
  const maxFolded = $derived(Math.max(0, crumbs.length - 2));
  const visibleTail = $derived(crumbs.slice(1 + folded));
  // Fully folded and still too wide: let the survivors share the
  // squeeze (legible floors) instead of hard-clipping the tail.
  let tight = $state(false);

  $effect(() => {
    void crumbs;
    folded = 0;
    tight = false;
  });

  $effect(() => {
    void visibleTail;
    if (!navEl) return;
    const over = navEl.scrollWidth > navEl.clientWidth + 1;
    if (over && folded < maxFolded) folded += 1;
    else if (over) tight = true;
  });

  // A width change re-derives the fold from scratch (so growing the
  // window unfolds). The observer's initial fire reports the width it
  // already has and stays a no-op.
  $effect(() => {
    if (!navEl) return;
    let lastWidth = navEl.clientWidth;
    const ro = new ResizeObserver(() => {
      if (!navEl) return;
      const w = navEl.clientWidth;
      if (w !== lastWidth) {
        lastWidth = w;
        folded = 0;
        tight = false;
      }
    });
    ro.observe(navEl);
    return () => ro.disconnect();
  });

  let draft = $state("");
  let inputRef = $state<HTMLInputElement | null>(null);

  $effect(() => {
    if (editing) {
      draft = path;
      tick().then(() => {
        inputRef?.focus();
        inputRef?.select();
      });
    }
  });

  function commit() {
    const target = draft.trim();
    editing = false;
    if (target && target !== path) onnavigate?.(target);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      editing = false;
    }
  }
</script>

{#if editing}
  <input
    bind:this={inputRef}
    bind:value={draft}
    class="bc-input"
    aria-label="Path"
    spellcheck="false"
    onkeydown={onkeydown}
    onblur={() => (editing = false)}
  />
{:else}
  <nav class="bc" class:tight aria-label="Path" bind:this={navEl}>
    {#if crumbs.length > 0}
      <button
        class="bc-crumb"
        class:current={crumbs.length === 1}
        onclick={() => onnavigate?.(crumbs[0].path)}
      >
        {crumbs[0].name}
      </button>
    {/if}
    {#if folded > 0}
      <span class="bc-sep" aria-hidden="true">
        <ChevronRight size={12} strokeWidth={2} />
      </span>
      <span class="bc-fold">…</span>
    {/if}
    {#each visibleTail as crumb, i (crumb.path)}
      <span class="bc-sep" aria-hidden="true">
        <ChevronRight size={12} strokeWidth={2} />
      </span>
      <button
        class="bc-crumb"
        class:current={i === visibleTail.length - 1}
        onclick={() => onnavigate?.(crumb.path)}
      >
        {crumb.name}
      </button>
    {/each}
  </nav>
{/if}

<style>
  .bc {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
  }
  /* Natural width per crumb (the fold handles overflow); only a
     single monster name ellipsizes on its own. */
  .bc-crumb {
    flex-shrink: 0;
    max-width: 12rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    height: var(--height-control-compact, 24px);
    padding: 0 6px;
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .bc-crumb:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .bc-crumb.current {
    color: var(--foreground);
    font-weight: 500;
  }
  .bc-sep {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
  .bc-fold {
    flex-shrink: 0;
    padding: 0 2px;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }
  .bc.tight .bc-crumb {
    flex-shrink: 1;
    min-width: 2.5rem;
  }

  .bc-input {
    width: 100%;
    height: var(--height-control, 28px);
    padding: 0 8px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-input);
    background: var(--color-bg-input, var(--background));
    color: var(--foreground);
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    outline: none;
  }
  .bc-input:focus-visible {
    border-color: var(--control-border-hover);
  }
</style>
