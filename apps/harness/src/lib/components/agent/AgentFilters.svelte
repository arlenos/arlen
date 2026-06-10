<script lang="ts">
  /// Agent surface contextual filter column (ai-app.md §2.0, dashboard
  /// archetype): scopes the activity timeline by type, outcome, and time
  /// window. Single-select per group; "All" clears it. Sits on the right as
  /// the dashboard's contextual pane.
  ///
  /// Options for type/outcome are data-driven (derived from the loaded
  /// entries by the page) rather than hardcoded, so whatever the ledger
  /// holds is filterable and nothing stale is offered. Behaviour-name
  /// filtering is deliberately absent: the audit entry is content-free and
  /// carries no behaviour name (same constraint as H-actor).
  interface Option {
    value: string;
    label: string;
  }

  let {
    kinds,
    outcomes,
    selectedKind = $bindable(null),
    selectedOutcome = $bindable(null),
    timeWindow = $bindable("all"),
  }: {
    kinds: Option[];
    outcomes: Option[];
    selectedKind: string | null;
    selectedOutcome: string | null;
    timeWindow: string;
  } = $props();

  const TIMES: Option[] = [
    { value: "all", label: "All time" },
    { value: "1h", label: "Last hour" },
    { value: "24h", label: "Last 24 hours" },
    { value: "7d", label: "Last 7 days" },
  ];
</script>

{#snippet group(
  label: string,
  options: Option[],
  selected: string | null,
  select: (v: string | null) => void,
  allValue: string | null,
)}
  <div class="filter-group">
    <span class="filter-label">{label}</span>
    {#if allValue === null}
      <button
        class="filter-item"
        class:active={selected === null}
        onclick={() => select(null)}
      >
        All
      </button>
    {/if}
    {#each options as o (o.value)}
      <button
        class="filter-item"
        class:active={selected === o.value}
        onclick={() => select(o.value)}
      >
        {o.label}
      </button>
    {/each}
  </div>
{/snippet}

<aside class="filters" aria-label="Activity filters">
  {@render group("Type", kinds, selectedKind, (v) => (selectedKind = v), null)}
  {#if outcomes.length > 0}
    {@render group("Outcome", outcomes, selectedOutcome, (v) => (selectedOutcome = v), null)}
  {/if}
  {@render group("Time", TIMES, timeWindow, (v) => (timeWindow = v ?? "all"), "all")}
  <p class="filter-note">Filters scope the activity timeline.</p>
</aside>

<style>
  .filters {
    display: flex;
    flex-direction: column;
    gap: var(--space-section, 1.5rem);
    width: 13rem;
    flex-shrink: 0;
    padding: var(--space-page, 1.5rem) var(--space-row, 0.75rem);
    border-left: 1px solid var(--color-border);
    position: sticky;
    top: 0;
    align-self: flex-start;
  }
  .filter-group {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }
  /* Same register as the Group label, so the column reads as part of the
     dashboard rather than a foreign sidebar. */
  .filter-label {
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    padding: 0 0.5rem 0.25rem;
  }
  .filter-item {
    display: flex;
    align-items: center;
    min-height: var(--height-control-compact, 24px);
    width: 100%;
    text-align: left;
    padding: 0 0.5rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    font-size: 0.8125rem;
    border-radius: var(--radius-chip);
    cursor: pointer;
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .filter-item:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .filter-item.active {
    background: color-mix(in srgb, var(--color-accent) 16%, transparent);
    color: var(--foreground);
  }
  .filter-note {
    margin: 0;
    padding: 0 0.5rem;
    font-size: 0.75rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  /* On a narrow window the filter column yields so the timeline keeps a
     usable width. */
  @media (max-width: 52rem) {
    .filters {
      display: none;
    }
  }
</style>
