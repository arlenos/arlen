<script lang="ts">
  /// Prep-for-this (agent-work-surfaces-plan.md surface 3): pick a live entity,
  /// pull everything the knowledge graph connects to it, ranked and grouped by
  /// liveness. Pure read, no gate - the cleanest "the KG earns its keep" demo.
  /// Live via `working_set_briefing` + `prep_for`; fixture-backed under vite.
  import { onMount } from "svelte";
  import { t, dir } from "$lib/i18n/messages";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { Compass, Search, ChevronDown } from "@lucide/svelte";
  import {
    workingSet,
    subject,
    prepped,
    loading,
    loadWorkingSet,
    prepFor,
    type PrepItem,
  } from "$lib/stores/prep";

  onMount(loadWorkingSet);

  let query = $state("");
  let staleOpen = $state(false);

  const picks = $derived(
    $workingSet.filter((i) => i.label.toLowerCase().includes(query.trim().toLowerCase())),
  );

  // The backend returns items score-ranked; grouping by liveness preserves order.
  const live = $derived($prepped.filter((i) => i.liveness === "live"));
  const dormant = $derived($prepped.filter((i) => i.liveness === "dormant"));
  const stale = $derived($prepped.filter((i) => i.liveness === "stale"));
</script>

<div class="prep" dir={$dir}>
  <header class="head">
    <div class="head-title"><Compass size={18} strokeWidth={2} aria-hidden="true" /> <h1>{$t("h.prep.title")}</h1></div>
    <p class="sub">{$t("h.prep.sub")}</p>
  </header>

  <div class="picker">
    <div class="filter">
      <Search size={13} strokeWidth={2} class="filter-icon" aria-hidden="true" />
      <Input bind:value={query} placeholder={$t("h.prep.filter")} aria-label={$t("h.prep.filter")} />
    </div>
    <div class="picks">
      {#each picks as it (it.id)}
        <button type="button" class="pick" class:active={$subject?.id === it.id} onclick={() => prepFor(it)}>
          <span class="pick-label">{it.label}</span>
          <Badge variant="secondary">{it.kind}</Badge>
          <span class="dot liveness-{it.liveness}" aria-hidden="true"></span>
        </button>
      {/each}
    </div>
  </div>

  {#if !$subject}
    <p class="pick-prompt">{$t("h.prep.pickPrompt")}</p>
  {:else}
    <section class="result">
      <h2 class="result-head">{$t("h.prep.preppedFor", { subject: $subject.label })}</h2>
      {#if $loading}
        <p class="muted">{$t("h.prep.loading")}</p>
      {:else if $prepped.length === 0}
        <p class="muted">{$t("h.prep.noContext", { subject: $subject.label })}</p>
      {:else}
        {#if live.length}
          {@render group($t("h.prep.groupLive"), live)}
        {/if}
        {#if dormant.length}
          {@render group($t("h.prep.groupDormant"), dormant)}
        {/if}
        {#if stale.length}
          <div class="group">
            <button type="button" class="group-toggle" onclick={() => (staleOpen = !staleOpen)} aria-expanded={staleOpen}>
              <ChevronDown size={13} strokeWidth={2} class={staleOpen ? "chev open" : "chev"} aria-hidden="true" />
              {$t("h.prep.staleToggle", { count: stale.length })}
            </button>
            {#if staleOpen}
              <ul class="cards">
                {#each stale as it (it.id)}{@render card(it)}{/each}
              </ul>
            {/if}
          </div>
        {/if}
      {/if}
    </section>
  {/if}
</div>

{#snippet group(label: string, items: PrepItem[])}
  <div class="group">
    <p class="group-label">{label}</p>
    <ul class="cards">
      {#each items as it (it.id)}{@render card(it)}{/each}
    </ul>
  </div>
{/snippet}

{#snippet card(it: PrepItem)}
  <li class="card">
    <span class="card-label">{it.label}</span>
    <Badge variant="secondary">{it.kind}</Badge>
    <span class="card-relation">{it.relation}</span>
    <span class="score" aria-hidden="true"><span class="score-fill" style="width:{Math.round(it.score * 100)}%"></span></span>
  </li>
{/snippet}

<style>
  .prep {
    height: 100%;
    overflow-y: auto;
    padding: 1.75rem 2rem;
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
    max-width: 52rem;
    margin-inline: auto;
    color: var(--foreground);
  }
  .head-title {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }
  .head-title h1 {
    margin: 0;
    font-size: var(--text-lg);
    font-weight: 600;
  }
  .sub {
    margin: 0.4rem 0 0;
    font-size: var(--text-sm);
    line-height: 1.45;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .picker {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }
  .filter {
    position: relative;
  }
  .filter :global(.filter-icon) {
    position: absolute;
    left: 0.65rem;
    top: 50%;
    transform: translateY(-50%);
    opacity: 0.5;
    pointer-events: none;
    z-index: 1;
  }
  .filter :global(input) {
    padding-inline-start: 1.75rem;
  }
  .picks {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .pick {
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.35rem 0.6rem;
    border: 1px solid color-mix(in srgb, var(--foreground) 12%, transparent);
    border-radius: var(--radius-input);
    background: transparent;
    font-size: var(--text-sm);
    color: var(--foreground);
    cursor: pointer;
    transition: background var(--duration-fast, 150ms) ease, border-color var(--duration-fast, 150ms) ease;
  }
  .pick:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .pick.active {
    border-color: color-mix(in srgb, var(--color-accent) 60%, transparent);
    background: color-mix(in srgb, var(--color-accent) 12%, transparent);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }
  .liveness-live {
    background: var(--color-success, #10b981);
  }
  .liveness-dormant {
    background: var(--color-warning, #f59e0b);
  }
  .liveness-stale {
    background: color-mix(in srgb, var(--foreground) 30%, transparent);
  }

  .pick-prompt {
    margin: 0;
    font-size: var(--text-base);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .result {
    display: flex;
    flex-direction: column;
    gap: 1.25rem;
  }
  .result-head {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
  }
  .muted {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  .group {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .group-label {
    margin: 0;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .group-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    align-self: flex-start;
    padding: 0;
    border: none;
    background: none;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    cursor: pointer;
  }
  .group-toggle :global(.chev) {
    transition: transform var(--duration-fast, 150ms) ease;
    transform: rotate(-90deg);
  }
  .group-toggle :global(.chev.open) {
    transform: rotate(0deg);
  }

  .cards {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
  }
  /* One baseline: label, kind, the relation phrase, and the score cue all sit on
     a single row so nothing floats. The relation takes the middle and truncates. */
  .card {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
    border-radius: var(--radius-input);
  }
  .card-label {
    flex-shrink: 0;
    max-width: 45%;
    font-size: var(--text-sm);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-relation {
    flex: 1;
    min-width: 0;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .score {
    flex-shrink: 0;
    width: 48px;
    height: 4px;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    overflow: hidden;
  }
  .score-fill {
    display: block;
    height: 100%;
    background: color-mix(in srgb, var(--color-accent) 70%, transparent);
  }
</style>
