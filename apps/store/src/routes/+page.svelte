<script lang="ts">
  /// The store browse (store-app.md §8.1/§8.7, content-forward): search first,
  /// the capability facets as honest toggle chips, then the hand-picked editorial
  /// collections. Discovery is plain and good - no engagement machinery, no
  /// stars, search runs local.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import StoreHeader from "$lib/components/StoreHeader.svelte";
  import StoreRail from "$lib/components/StoreRail.svelte";
  import AppCard from "$lib/components/AppCard.svelte";
  import { t } from "$lib/i18n/messages";
  import {
    apps,
    catalogMocked,
    loadCatalog,
    COLLECTIONS,
    facetFlags,
    trustOf,
    defaultVariantOf,
    type StoreApp,
  } from "$lib/stores/catalog";

  onMount(loadCatalog);

  let query = $state("");

  // The capability facets (§8.1): the vocabulary is the real grant classes.
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
    let list = visible.filter((a) => a.name.toLowerCase().includes(query.trim().toLowerCase()));
    for (const f of active) list = list.filter((a) => facetFlags(a)[f]);
    if (leastPrivilege) list = [...list].sort((a, b) => defaultVariantOf(a).capWeight - defaultVariantOf(b).capWeight);
    return list;
  });

  function byId(id: string): StoreApp | undefined {
    return visible.find((a) => a.id === id);
  }
  function open(id: string): void {
    void goto(`/app/${id}`);
  }
</script>

<div class="st-app">
  <StoreHeader />

  <div class="st-body">
  <StoreRail />
  <main class="st-main">
  <div class="st-content">
    {#if $catalogMocked}
      <p class="sample">{$t("st.sample")}</p>
    {/if}

    <div class="search">
      <Input bind:value={query} placeholder={$t("st.search")} aria-label={$t("st.search")} id="store-search" />
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
      {#each COLLECTIONS as coll (coll.labelKey)}
        {@const members = coll.ids.map(byId).filter((a): a is StoreApp => a !== undefined)}
        {#if members.length > 0}
          <div class="group-label">{$t(coll.labelKey)}</div>
          <div class="grid">
            {#each members as app (app.id)}
              <AppCard {app} onopen={open} />
            {/each}
          </div>
        {/if}
      {/each}
    {/if}
  </div>
  </main>
  </div>
</div>

<style>
  .st-app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--color-bg-app);
    color: var(--color-fg-primary);
  }
  .st-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  /* The scroller spans the content area so the scrollbar sits at the edge; the
     content column is capped inside it. */
  .st-main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .st-content {
    width: 100%;
    max-width: 46rem;
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
    cursor: pointer;
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
  .quiet {
    margin: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
</style>
