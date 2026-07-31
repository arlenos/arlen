<script lang="ts">
  /// The search surface (KA-R4): results for the titlebar query, refined by
  /// GUIDED facets (type, project, time - the SemFacet form, never a query
  /// language), each hit labelled by its node type. One honest line says the
  /// match is by name (decision 2); the by-meaning group arrives only with a
  /// verified retrieval backend. Without a query the saved searches stand
  /// here - the query-as-folder bet - and Save keeps the current state as one.
  import { X, ChevronDown, Bookmark } from "lucide-svelte";
  import {
    query,
    facets,
    results,
    searchMocked,
    savedSearches,
    saveSearch,
    runSaved,
    projectChoices,
    type ResultType,
    type SearchResult,
  } from "$lib/stores/search";
  import { clock } from "$lib/stores/timeline";
  import { t, locale } from "$lib/i18n/messages";

  let { onselect }: { onselect: (r: SearchResult) => void } = $props();

  const TYPES: ResultType[] = ["file", "project", "paper", "mail", "note", "session"];
  const TIMES = [
    { days: 1, key: "k.se.today" },
    { days: 7, key: "k.se.week" },
    { days: 30, key: "k.se.month" },
  ];

  function typeLabel(type: ResultType): string {
    return $t(`k.se.type.${type}`);
  }

  let openFacet = $state<"type" | "project" | "time" | null>(null);
  function toggleFacet(which: "type" | "project" | "time"): void {
    openFacet = openFacet === which ? null : which;
  }
  function setType(v: ResultType | null): void {
    facets.update((f) => ({ ...f, type: v }));
    openFacet = null;
  }
  function setProject(v: string | null): void {
    facets.update((f) => ({ ...f, project: v }));
    openFacet = null;
  }
  function setTime(v: number | null): void {
    facets.update((f) => ({ ...f, withinDays: v }));
    openFacet = null;
  }

  let saving = $state(false);
  let saveName = $state("");
  async function confirmSave(): Promise<void> {
    const name = saveName.trim() || $query.trim();
    if (!name) return;
    await saveSearch(name);
    saving = false;
    saveName = "";
  }

  const hasState = $derived(
    $query.trim().length > 0 || $facets.type !== null || $facets.project !== null || $facets.withinDays !== null
  );

  function dayName(at: number | undefined): string {
    if (!at) return "";
    return new Date(at * 1000).toLocaleDateString($locale, { day: "numeric", month: "short" });
  }
</script>

<div class="se">
  <div class="se-head">
    {#if $searchMocked && hasState}
      <span class="se-sample">{$t("k.sample")}</span>
    {/if}
    <span class="se-spacer"></span>
    <!-- Guided refinement: three facet chips, each a small picker. -->
    <div class="se-facets">
      <div class="se-facet-wrap">
        {#if $facets.type === null}
          <button type="button" class="se-facet" onclick={() => toggleFacet("type")}>
            {$t("k.se.facet.type")}
            <ChevronDown size={12} strokeWidth={2} />
          </button>
        {:else}
          <span class="se-facet active">
            {typeLabel($facets.type)}
            <button type="button" class="se-facet-x" onclick={() => setType(null)} aria-label={$t("k.se.clearFacet")}>
              <X size={11} strokeWidth={2} />
            </button>
          </span>
        {/if}
        {#if openFacet === "type"}
          <div class="se-menu" role="listbox">
            {#each TYPES as ty (ty)}
              <button type="button" class="se-option" role="option" aria-selected="false" onclick={() => setType(ty)}>
                {typeLabel(ty)}
              </button>
            {/each}
          </div>
        {/if}
      </div>

      <div class="se-facet-wrap">
        {#if $facets.project === null}
          <button type="button" class="se-facet" onclick={() => toggleFacet("project")}>
            {$t("k.se.facet.project")}
            <ChevronDown size={12} strokeWidth={2} />
          </button>
        {:else}
          <span class="se-facet active">
            {$facets.project}
            <button type="button" class="se-facet-x" onclick={() => setProject(null)} aria-label={$t("k.se.clearFacet")}>
              <X size={11} strokeWidth={2} />
            </button>
          </span>
        {/if}
        {#if openFacet === "project"}
          <div class="se-menu" role="listbox">
            {#each projectChoices() as p (p)}
              <button type="button" class="se-option" role="option" aria-selected="false" onclick={() => setProject(p)}>
                {p}
              </button>
            {/each}
          </div>
        {/if}
      </div>

      <div class="se-facet-wrap">
        {#if $facets.withinDays === null}
          <button type="button" class="se-facet" onclick={() => toggleFacet("time")}>
            {$t("k.se.facet.time")}
            <ChevronDown size={12} strokeWidth={2} />
          </button>
        {:else}
          <span class="se-facet active">
            {$t($facets.withinDays === 1 ? "k.se.today" : $facets.withinDays === 7 ? "k.se.week" : "k.se.month")}
            <button type="button" class="se-facet-x" onclick={() => setTime(null)} aria-label={$t("k.se.clearFacet")}>
              <X size={11} strokeWidth={2} />
            </button>
          </span>
        {/if}
        {#if openFacet === "time"}
          <div class="se-menu" role="listbox">
            {#each TIMES as tm (tm.days)}
              <button type="button" class="se-option" role="option" aria-selected="false" onclick={() => setTime(tm.days)}>
                {$t(tm.key)}
              </button>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  </div>

  <div class="se-scroll">
    {#if hasState}
      <div class="se-meta">
        <span class="se-matchline">{$t("k.se.byName")}</span>
        <span class="se-spacer"></span>
        {#if saving}
          <input
            type="text"
            class="se-save-name"
            bind:value={saveName}
            placeholder={$query.trim() || $t("k.se.saveNamePlaceholder")}
            onkeydown={(e) => {
              if (e.key === "Enter") void confirmSave();
              if (e.key === "Escape") (saving = false), (saveName = "");
            }}
          />
          <button type="button" class="se-save" onclick={confirmSave}>{$t("k.se.saveConfirm")}</button>
        {:else}
          <button type="button" class="se-save" onclick={() => (saving = true)}>
            <Bookmark size={12} strokeWidth={2} />
            {$t("k.se.save")}
          </button>
        {/if}
      </div>

      {#if $results.length === 0}
        <p class="se-empty">{$t("k.se.none")}</p>
      {:else}
        <div class="se-results">
          {#each $results as r (r.id)}
            <button type="button" class="se-row" onclick={() => onselect(r)}>
              <span class="se-type">{typeLabel(r.type)}</span>
              <span class="se-title">{r.title}</span>
              <span class="se-sub">
                {r.sub}{#if r.project}<span class="se-proj">{r.project}</span>{/if}
              </span>
              <span class="se-time">{dayName(r.at)}{#if r.at}, {clock(r.at, $locale)}{/if}</span>
            </button>
          {/each}
        </div>
      {/if}
    {:else}
      <h2 class="se-saved-head">{$t("k.se.saved")}</h2>
      {#if $savedSearches.length === 0}
        <p class="se-empty">{$t("k.empty.searches")}</p>
      {:else}
        <div class="se-results">
          {#each $savedSearches as s (s.id)}
            <button type="button" class="se-row" onclick={() => runSaved(s)}>
              <span class="se-type saved"><Bookmark size={11} strokeWidth={2} /></span>
              <span class="se-title">{s.name}</span>
              <span class="se-sub">
                {#if s.query}"{s.query}"{/if}
                {#if s.facets.project}<span class="se-proj">{s.facets.project}</span>{/if}
                {#if s.facets.type}<span class="se-proj">{typeLabel(s.facets.type)}</span>{/if}
              </span>
              <span class="se-time"></span>
            </button>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
</div>

<style>
  .se {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .se-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.6rem 1.1rem 0.45rem;
  }
  .se-sample {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .se-spacer {
    flex: 1;
  }

  .se-facets {
    display: flex;
    align-items: center;
    gap: 0.375rem;
  }
  .se-facet-wrap {
    position: relative;
  }
  .se-facet {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.2rem 0.5rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-button, 6px);
    background: transparent;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 65%, transparent);
    cursor: pointer;
  }
  .se-facet:hover {
    color: var(--color-fg-primary);
  }
  .se-facet.active {
    border-color: color-mix(in srgb, var(--color-accent, #6aa9e0) 40%, transparent);
    background: color-mix(in srgb, var(--color-accent, #6aa9e0) 10%, transparent);
    color: var(--color-fg-primary);
    cursor: default;
  }
  .se-facet-x {
    display: inline-flex;
    padding: 0.1rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    cursor: pointer;
  }
  .se-facet-x:hover {
    color: var(--color-fg-primary);
  }
  .se-menu {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    min-width: 9rem;
    padding: 0.25rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
    border-radius: var(--radius-input);
    background: var(--color-bg-card);
    box-shadow: var(--shadow-lg);
  }
  .se-option {
    padding: 0.35rem 0.5rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    font-size: var(--text-xs);
    color: var(--color-fg-primary);
    text-align: start;
    cursor: pointer;
  }
  .se-option:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 7%, transparent);
  }

  .se-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0.25rem 1.1rem 1.25rem;
  }
  /* The honesty line: how these results matched, said once. */
  .se-meta {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0.25rem 0 0.5rem;
  }
  .se-matchline {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .se-save {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.2rem 0.5rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-button, 6px);
    background: transparent;
    font-size: var(--text-xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 65%, transparent);
    cursor: pointer;
  }
  .se-save:hover {
    color: var(--color-fg-primary);
  }
  .se-save-name {
    width: 12rem;
    height: 1.6rem;
    padding: 0 0.5rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
    font-size: var(--text-xs);
    color: var(--color-fg-primary);
    outline: none;
  }

  .se-empty {
    margin: 0.75rem 0 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .se-saved-head {
    margin: 0.6rem 0 0.25rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }

  /* Result rows: type tag, emphasized title, quiet context, tabular time. */
  .se-results {
    display: flex;
    flex-direction: column;
  }
  /* The time column is FIXED so the two fr columns resolve identically on
     every row - a max-content time made the sub column drift a few pixels
     between rows (each row is its own grid). */
  .se-row {
    display: grid;
    grid-template-columns: 4.5rem minmax(0, 1.2fr) minmax(0, 1fr) 7.5rem;
    align-items: baseline;
    column-gap: 0.75rem;
    width: 100%;
    padding: 0.35rem 0.375rem;
    border: none;
    border-radius: var(--radius-chip, 4px);
    background: transparent;
    text-align: start;
    cursor: pointer;
  }
  .se-row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .se-type {
    justify-self: start;
    padding: 0.0625rem 0.375rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .se-type.saved {
    display: inline-flex;
    align-self: center;
    padding: 0.2rem;
  }
  .se-title {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .se-sub {
    display: inline-flex;
    align-items: baseline;
    gap: 0.4rem;
    min-width: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .se-proj {
    padding: 0.0625rem 0.375rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 7%, transparent);
    font-size: var(--text-2xs);
    font-weight: 500;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .se-time {
    justify-self: end;
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    white-space: nowrap;
  }
</style>
