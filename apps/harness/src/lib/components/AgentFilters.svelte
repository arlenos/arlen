<script lang="ts">
  /// Agent surface contextual filter column (ai-app.md §2.0, dashboard
  /// archetype's filter column). Scopes the activity timeline by type,
  /// outcome, and time window. Single-select per group; "All" clears it.
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

<aside class="filters" aria-label="Activity filters">
  <div class="filter-group">
    <span class="filter-label">Type</span>
    <button class="filter-item" class:active={selectedKind === null} onclick={() => (selectedKind = null)}>
      All
    </button>
    {#each kinds as k (k.value)}
      <button
        class="filter-item"
        class:active={selectedKind === k.value}
        onclick={() => (selectedKind = k.value)}
      >
        {k.label}
      </button>
    {/each}
  </div>

  {#if outcomes.length > 0}
    <div class="filter-group">
      <span class="filter-label">Outcome</span>
      <button class="filter-item" class:active={selectedOutcome === null} onclick={() => (selectedOutcome = null)}>
        All
      </button>
      {#each outcomes as o (o.value)}
        <button
          class="filter-item"
          class:active={selectedOutcome === o.value}
          onclick={() => (selectedOutcome = o.value)}
        >
          {o.label}
        </button>
      {/each}
    </div>
  {/if}

  <div class="filter-group">
    <span class="filter-label">Time</span>
    {#each TIMES as t (t.value)}
      <button
        class="filter-item"
        class:active={timeWindow === t.value}
        onclick={() => (timeWindow = t.value)}
      >
        {t.label}
      </button>
    {/each}
  </div>

  <p class="filter-note">Filters scope the activity timeline.</p>
</aside>

<style>
  .filters {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    width: 13rem;
    flex-shrink: 0;
    padding: 0.85rem 0.7rem;
    border-right: 1px solid var(--color-border);
    background: color-mix(in srgb, var(--color-bg-card) 35%, transparent);
    position: sticky;
    top: 0;
    align-self: flex-start;
    max-height: 100%;
    overflow-y: auto;
  }
  .filter-group {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }
  .filter-label {
    font-size: 0.68rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    margin-bottom: 0.2rem;
    padding: 0 0.4rem;
  }
  .filter-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 0.3rem 0.4rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    font-size: 0.8rem;
    border-radius: var(--radius-chip);
    cursor: pointer;
  }
  .filter-item:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .filter-item.active {
    background: color-mix(in srgb, var(--color-accent) 16%, transparent);
    color: var(--foreground);
  }
  .filter-note {
    margin: 0;
    padding: 0 0.4rem;
    font-size: 0.7rem;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
  @media (max-width: 52rem) {
    .filters {
      display: none;
    }
  }
</style>
