<script lang="ts">
  /// The "Filter" dropdown in the search row: the type and time facets
  /// (each a submenu of choices), the content-match toggle and "save this
  /// search", collapsed out of the row so it reads as [query] Filter X
  /// instead of a line of pills. The trigger carries a dot whenever a facet
  /// differs from its default, so a narrowed search never hides its state.
  import { ChevronDown } from "@lucide/svelte";
  import * as DropdownMenu from "@arlen/ui-kit/components/ui/dropdown-menu";
  import {
    runSearch,
    searchContent,
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

  const typeLabel = $derived(
    TYPE_OPTIONS.find((o) => o.value === $searchType)?.label ?? "Any type",
  );
  const timeLabel = $derived(
    TIME_OPTIONS.find((o) => o.value === $searchTime)?.label ?? "Any time",
  );
  // Any facet narrowed from its default lights the trigger dot.
  const active = $derived(
    $searchType !== "any" || $searchTime !== "any" || $searchContent,
  );
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      <button class="filter-trigger" aria-label="Filter results" {...props}>
        <span>Filter</span>
        {#if active}
          <span class="dot" aria-hidden="true"></span>
        {/if}
        <ChevronDown size={12} strokeWidth={2} class="chev" />
      </button>
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Content align="end" sideOffset={4} class="fm-menu fm-menu-checks">
    <DropdownMenu.Sub>
      <DropdownMenu.SubTrigger>
        Type
        {#if $searchType !== "any"}
          <span class="hint">{typeLabel}</span>
        {/if}
      </DropdownMenu.SubTrigger>
      <DropdownMenu.SubContent class="fm-menu">
        <DropdownMenu.RadioGroup
          value={$searchType}
          onValueChange={(v) => {
            searchType.set(v as TypeFacet);
            void runSearch(path);
          }}
        >
          {#each TYPE_OPTIONS as opt (opt.value)}
            <DropdownMenu.RadioItem value={opt.value}>
              {opt.label}
            </DropdownMenu.RadioItem>
          {/each}
        </DropdownMenu.RadioGroup>
      </DropdownMenu.SubContent>
    </DropdownMenu.Sub>

    <DropdownMenu.Sub>
      <DropdownMenu.SubTrigger>
        Time
        {#if $searchTime !== "any"}
          <span class="hint">{timeLabel}</span>
        {/if}
      </DropdownMenu.SubTrigger>
      <DropdownMenu.SubContent class="fm-menu">
        <DropdownMenu.RadioGroup
          value={$searchTime}
          onValueChange={(v) => {
            searchTime.set(v as TimeFacet);
            void runSearch(path);
          }}
        >
          {#each TIME_OPTIONS as opt (opt.value)}
            <DropdownMenu.RadioItem value={opt.value}>
              {opt.label}
            </DropdownMenu.RadioItem>
          {/each}
        </DropdownMenu.RadioGroup>
      </DropdownMenu.SubContent>
    </DropdownMenu.Sub>

    <DropdownMenu.Separator />

    <DropdownMenu.CheckboxItem
      checked={$searchContent}
      closeOnSelect={false}
      onSelect={() => {
        searchContent.update((v) => !v);
        void runSearch(path);
      }}
    >
      Search inside file contents
    </DropdownMenu.CheckboxItem>

    <DropdownMenu.Separator />

    <DropdownMenu.Item
      disabled={$searchQuery.trim().length === 0}
      onSelect={() => onsave?.($searchQuery.trim())}
    >
      Save this search
    </DropdownMenu.Item>
  </DropdownMenu.Content>
</DropdownMenu.Root>

<style>
  .filter-trigger {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
    height: var(--height-control, 28px);
    padding: 0 10px;
    border: 1px solid var(--control-border);
    background: var(--control-bg);
    border-radius: var(--radius-input);
    color: var(--foreground);
    font-size: 0.75rem;
    font-weight: 500;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out);
  }
  .filter-trigger:hover,
  .filter-trigger[aria-expanded="true"] {
    background: var(--control-bg-hover);
  }
  .filter-trigger .dot {
    width: 5px;
    height: 5px;
    border-radius: var(--radius-chip);
    background: var(--color-accent);
  }
  .filter-trigger :global(.chev) {
    opacity: 0.55;
  }
  .hint {
    margin-left: auto;
    padding-left: 1rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  /* This menu carries a checkbox row, so the submenu triggers and the
     plain "Save" item reserve the same leading check column — every
     label lines up under one edge (the macOS / GNOME convention). */
  :global(.fm-menu-checks [data-slot="dropdown-menu-item"]),
  :global(.fm-menu-checks [data-slot="dropdown-menu-sub-trigger"]) {
    padding-left: 2rem;
  }
</style>
