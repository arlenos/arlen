<script lang="ts">
  /// The search row under the toolbar: query plus the two facet
  /// filters in the family pattern (PopoverSelects in a controls
  /// row). Results render in place of the listing; saving as a place
  /// keeps the search in the sidebar for this session (persistence
  /// needs a contract command, flagged).
  import { tick } from "svelte";
  import { X } from "lucide-svelte";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import {
    closeSearch,
    queueSearch,
    runSearch,
    searchContent,
    searchOpen,
    searchQuery,
    searchTime,
    searchType,
    type TimeFacet,
    type TypeFacet,
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

  const TYPE_OPTIONS: { value: TypeFacet; label: string }[] = [
    { value: "any", label: "Any type" },
    { value: "folder", label: "Folders" },
    { value: "document", label: "Documents" },
    { value: "image", label: "Images" },
    { value: "audio", label: "Audio" },
    { value: "video", label: "Video" },
    { value: "archive", label: "Archives" },
    { value: "code", label: "Code" },
  ];
  const TIME_OPTIONS: { value: TimeFacet; label: string }[] = [
    { value: "any", label: "Any time" },
    { value: "day", label: "Today" },
    { value: "week", label: "Last 7 days" },
    { value: "month", label: "Last 30 days" },
  ];

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
    <PopoverSelect
      ariaLabel="Filter by type"
      width="8.5rem"
      options={TYPE_OPTIONS}
      value={$searchType}
      onchange={(v) => {
        searchType.set(v as TypeFacet);
        void runSearch(path);
      }}
    />
    <PopoverSelect
      ariaLabel="Filter by time"
      width="8.5rem"
      options={TIME_OPTIONS}
      value={$searchTime}
      onchange={(v) => {
        searchTime.set(v as TimeFacet);
        void runSearch(path);
      }}
    />
    <button
      class="sb-toggle"
      class:active={$searchContent}
      aria-pressed={$searchContent}
      title="Also search inside file contents"
      onclick={() => {
        searchContent.update((v) => !v);
        void runSearch(path);
      }}
    >
      Contents
    </button>
    <button
      class="sb-save"
      disabled={$searchQuery.trim().length === 0}
      onclick={() => onsave?.($searchQuery.trim())}
    >
      Save search
    </button>
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
    padding: 0 8px 8px;
  }
  .search-bar :global(input) {
    flex: 1;
  }

  .sb-save {
    flex-shrink: 0;
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
  .sb-save:hover:not(:disabled) {
    background: var(--control-bg-hover);
  }
  .sb-save:disabled {
    opacity: var(--control-disabled-opacity, 0.5);
  }

  .sb-toggle {
    flex-shrink: 0;
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
  .sb-toggle:hover {
    background: var(--control-bg-hover);
  }
  .sb-toggle.active {
    background: var(--color-accent);
    border-color: var(--color-accent);
    color: var(--color-accent-foreground, #fff);
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
