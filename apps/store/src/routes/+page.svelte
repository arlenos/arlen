<script lang="ts">
  /// The store browse (store-app.md §8.1/§8.7, content-forward): search first,
  /// the capability facets as honest toggle chips, the hand-picked editorial
  /// collections, then the whole catalogue. Discovery is plain and good - no
  /// engagement machinery, no stars, search runs local.
  ///
  /// The catalogue can be thousands of cards, so the all-apps grid renders in
  /// slices behind a "Show more" button - a button, not an infinite scroll,
  /// because the page should have an end a person can reach on purpose.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import AppCard from "$lib/components/AppCard.svelte";
  import { t, locale } from "$lib/i18n/messages";
  import {
    apps,
    collections,
    catalogMocked,
    loadCatalog,
    collectionTitle,
    trustOf,
    type StoreCard,
  } from "$lib/stores/catalog";

  onMount(loadCatalog);

  let query = $state("");

  // The capability facets (§8.1): the vocabulary is the real grant classes,
  // answered by the backend as per-card booleans - nothing is derived from
  // prose here.
  type FacetKey = "noNetwork" | "offlineOnly" | "noGraph" | "verified" | "reproducible";
  const FACETS: { key: FacetKey; labelKey: string }[] = [
    { key: "noNetwork", labelKey: "st.facet.noNetwork" },
    { key: "offlineOnly", labelKey: "st.facet.offline" },
    { key: "noGraph", labelKey: "st.facet.noGraph" },
    { key: "verified", labelKey: "st.facet.verified" },
    { key: "reproducible", labelKey: "st.facet.reproducible" },
  ];
  let active = $state<Set<FacetKey>>(new Set());
  let leastPrivilege = $state(false);
  // Default-safe (§8.1): community-tier apps are hidden until revealed.
  let showCommunity = $state(false);
  function toggleFacet(key: FacetKey): void {
    const next = new Set(active);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    active = next;
  }

  const visible = $derived($apps.filter((a) => showCommunity || trustOf(a) !== "community"));
  const filtering = $derived(query.trim().length > 0 || active.size > 0 || leastPrivilege);
  const results = $derived.by(() => {
    const q = query.trim().toLowerCase();
    let list = visible.filter(
      (a) => a.name.toLowerCase().includes(q) || a.summary.toLowerCase().includes(q) || a.id.toLowerCase().includes(q),
    );
    for (const f of active) list = list.filter((a) => a[f]);
    if (leastPrivilege) list = [...list].sort((a, b) => a.capWeight - b.capWeight);
    return list;
  });

  // The collections that actually have members in this catalogue. An empty one
  // is hidden; when none match at all (a catalogue the curator has not written
  // for yet), the page says so once and the full grid below carries the load.
  const liveCollections = $derived(
    $collections
      .map((c) => ({
        coll: c,
        members: c.members
          .map((id) => visible.find((a) => a.id === id))
          .filter((a): a is StoreCard => a !== undefined),
      }))
      .filter((c) => c.members.length > 0),
  );

  const everything = $derived([...visible].sort((a, b) => a.name.localeCompare(b.name)));

  // The all-apps slice. Resets when the reveal toggle changes the population.
  const SLICE = 60;
  let shown = $state(SLICE);
  $effect(() => {
    void showCommunity;
    shown = SLICE;
  });

  function open(id: string): void {
    void goto(`/app/${id}`);
  }
</script>

<main class="st-main">
  <div class="st-content">
    {#if $catalogMocked}
      <p class="sample">{$t("st.sample")}</p>
    {/if}

    <div class="search">
      <SearchField
        bind:value={query}
        size="prominent"
        placeholder={$t("st.search")}
        aria-label={$t("st.search")}
        id="store-search"
      />
    </div>

    <div class="facets" role="group" aria-label={$t("st.search")}>
      {#each FACETS as f (f.key)}
        <button
          type="button"
          class="chip"
          class:on={active.has(f.key)}
          aria-pressed={active.has(f.key)}
          id={`facet-${f.key}`}
          onclick={() => toggleFacet(f.key)}
        >
          {$t(f.labelKey)}
        </button>
      {/each}
      <button
        type="button"
        class="chip"
        class:on={leastPrivilege}
        aria-pressed={leastPrivilege}
        id="facet-least-privilege"
        onclick={() => (leastPrivilege = !leastPrivilege)}
      >
        {$t("st.sort.leastPrivilege")}
      </button>
      <button
        type="button"
        class="chip"
        class:on={showCommunity}
        aria-pressed={showCommunity}
        id="facet-community"
        onclick={() => (showCommunity = !showCommunity)}
      >
        {$t("st.facet.community")}
      </button>
    </div>

    {#if filtering}
      <div class="group-label">{$t("st.results")}</div>
      <div class="grid">
        {#each results as app (app.id)}
          <AppCard {app} onopen={open} />
        {:else}
          <p class="quiet">{$t("st.noMatch")}</p>
        {/each}
      </div>
    {:else}
      {#each liveCollections as { coll, members } (coll.id)}
        <div class="group-label">{collectionTitle(coll, $locale)}</div>
        <div class="grid">
          {#each members as app (app.id)}
            <AppCard {app} onopen={open} />
          {/each}
        </div>
      {/each}
      {#if liveCollections.length === 0 && everything.length > 0}
        <p class="quiet">{$t("st.coll.none")}</p>
      {/if}

      {#if everything.length > 0}
        <div class="group-label">{$t("st.all")}</div>
        <div class="grid">
          {#each everything.slice(0, shown) as app (app.id)}
            <AppCard {app} onopen={open} />
          {/each}
        </div>
        {#if everything.length > shown}
          <div class="more">
            <span class="count">{$t("st.shown", { n: shown, total: everything.length })}</span>
            <Button variant="outline" size="sm" id="store-show-more" onclick={() => (shown += SLICE * 2)}>
              {$t("st.showMore")}
            </Button>
          </div>
        {/if}
      {/if}
    {/if}
  </div>
</main>

<style>
  .st-main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .st-content {
    width: 100%;
    max-width: 64rem;
    margin: 0 auto;
    padding: 1.25rem 1.5rem 2rem;
  }
  .sample {
    margin: 0 0 0.75rem;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .search {
    margin-bottom: 0.75rem;
  }
  .facets {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-bottom: 1.5rem;
  }
  /* Honest toggle chips in the quiet-chip language; active = a stronger fill. */
  .chip {
    padding: 0.25rem 0.7rem;
    border: none;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    transition: background var(--duration-fast, 150ms) ease, color var(--duration-fast, 150ms) ease;
  }
  .chip:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 13%, transparent);
  }
  .chip.on {
    background: color-mix(in srgb, var(--color-fg-primary) 88%, transparent);
    color: var(--color-fg-inverse);
  }
  .group-label {
    margin: 1.25rem 0 0.6rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(13.5rem, 1fr));
    gap: 0.6rem;
  }
  .more {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    margin-top: 1rem;
  }
  .count {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .quiet {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
</style>
