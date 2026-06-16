<script lang="ts">
  /// The search row under the headerbar: the query field and the
  /// collapsed "Filter" dropdown (type, time, content match, save). Results
  /// render in place of the listing; saving as a place keeps the search in
  /// the sidebar for this session (persistence needs a contract command,
  /// flagged).
  import { tick } from "svelte";
  import { X } from "lucide-svelte";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import FmFilterMenu from "$lib/components/FmFilterMenu.svelte";
  import {
    closeSearch,
    queueSearch,
    searchOpen,
    searchQuery,
  } from "$lib/stores/search";

  let {
    path,
    onsave,
  }: {
    /// The location the search runs under.
    path: string;
    /// Save the current query as a sidebar search.
    onsave?: (query: string) => void;
  } = $props();

  let inputRef = $state<HTMLInputElement | null>(null);
  $effect(() => {
    if ($searchOpen) {
      tick().then(() => inputRef?.focus());
    }
  });
</script>

{#if $searchOpen}
  <div class="search-bar">
    <Input
      id="files-search-input"
      bind:ref={inputRef}
      bind:value={$searchQuery}
      class="h-7 text-xs"
      placeholder="Search this folder and everything inside it"
      aria-label="Search"
      oninput={() => queueSearch(path)}
      onkeydown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          closeSearch();
        }
      }}
    />
    <FmFilterMenu {path} {onsave} />
    <button class="sb-close" aria-label="Close search" onclick={() => closeSearch()}>
      <X size={14} strokeWidth={2} />
    </button>
  </div>
{/if}

<style>
  .search-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 8px;
  }
  .search-bar :global(input) {
    flex: 1;
  }
  /* Flat-house focus: the kit Input's accent glow ring is replaced by a
     quiet border shift, the register the rest of the chrome uses. */
  .search-bar :global(input:focus-visible) {
    box-shadow: none;
    border-color: color-mix(in srgb, var(--foreground) 35%, transparent);
  }

  .sb-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control, 28px);
    height: var(--height-control, 28px);
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .sb-close:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
</style>
