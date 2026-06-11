<script lang="ts">
  /// The path bar: clickable crumbs, and an editable path field
  /// behind Ctrl+L (the host toggles `editing`). A `homePath` prop
  /// collapses the home prefix into one "Home" crumb so deep paths
  /// read the way the places sidebar speaks.
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
  <nav class="bc" aria-label="Path">
    {#each crumbs as crumb, i (crumb.path)}
      {#if i > 0}
        <span class="bc-sep" aria-hidden="true">
          <ChevronRight size={12} strokeWidth={2} />
        </span>
      {/if}
      <button
        class="bc-crumb"
        class:current={i === crumbs.length - 1}
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
  .bc-crumb {
    flex-shrink: 1;
    min-width: 0;
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
    flex-shrink: 0;
    color: var(--foreground);
    font-weight: 500;
  }
  .bc-sep {
    display: inline-flex;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 35%, transparent);
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
