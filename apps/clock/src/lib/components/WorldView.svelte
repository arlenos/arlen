<script lang="ts">
  /// World clocks (clock-app.md §0.5): deliberately cheap. City rows over the
  /// shared offline dataset (a seam; a small fixture list drives the surface),
  /// local time + offset + day shift, add via the shared SearchField.
  import { X } from "lucide-svelte";
  import { SearchField } from "@arlen/ui-kit/components/ui/search-field";
  import { clock, tick, addCity, removeCity, CITY_DATASET } from "$lib/stores/clock";
  import { zoneTime, zoneOffsetHours, zoneDayShift } from "$lib/format";
  import { t, locale } from "$lib/i18n/messages";

  let query = $state("");
  const matches = $derived(
    query.trim().length === 0
      ? []
      : CITY_DATASET.filter(
          (c) =>
            c.name.toLowerCase().includes(query.trim().toLowerCase()) &&
            !$clock?.world.some((w) => w.id === c.id)
        ).slice(0, 5)
  );

  function offsetLine(zone: string, now: number): string {
    const h = zoneOffsetHours(zone, now);
    const shift = zoneDayShift(zone, now);
    const day = shift === 0 ? $t("c.wo.today") : shift > 0 ? $t("c.wo.tomorrow") : $t("c.wo.yesterday");
    const hours = Math.abs(h) % 1 === 0 ? String(Math.abs(h)) : Math.abs(h).toFixed(1);
    return $t("c.wo.offset", { sign: h >= 0 ? "+" : "-", hours, day });
  }
</script>

<div class="wo">
  <div class="wo-add">
    <SearchField id="city-search" bind:value={query} placeholder={$t("c.wo.search")} aria-label={$t("c.wo.search")} />
    {#if matches.length > 0}
      <div class="wo-matches">
        {#each matches as c (c.id)}
          <button
            type="button"
            class="wo-match"
            onclick={() => {
              void addCity(c);
              query = "";
            }}
          >
            <span>{c.name}</span>
            <span class="wo-match-time">{zoneTime(c.zone, $locale, $tick)}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>

  {#if $clock}
    {#if $clock.world.length === 0}
      <p class="wo-empty">{$t("c.wo.empty")}</p>
    {:else}
      <div class="wo-list">
        {#each $clock.world as w (w.id)}
          <div class="wo-row">
            <span class="wo-name">{w.name}</span>
            <span class="wo-offset">{offsetLine(w.zone, $tick)}</span>
            <span class="wo-time">{zoneTime(w.zone, $locale, $tick)}</span>
            <button type="button" class="wo-remove" aria-label={$t("c.wo.remove", { city: w.name })} onclick={() => removeCity(w.id)}>
              <X size={14} strokeWidth={2} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .wo {
    display: flex;
    flex-direction: column;
    gap: 0.9rem;
    max-width: 34rem;
    padding: 0.9rem 1rem 1.5rem;
  }
  .wo-add {
    position: relative;
    max-width: 18rem;
  }
  .wo-matches {
    position: absolute;
    top: calc(100% + 4px);
    inset-inline: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    padding: 4px;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
    border-radius: var(--radius-input);
    /* Concentric: the items step down from the menu radius by its padding. */
    --container-radius: var(--radius-input);
    --container-inset: 4px;
    background: var(--color-bg-card, #14161c);
    box-shadow: var(--shadow-lg);
  }
  .wo-match {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    padding: 0.45rem 0.6rem;
    border: none;
    border-radius: max(0px, calc(var(--container-radius) - var(--container-inset)));
    background: transparent;
    font-size: var(--text-sm);
    color: var(--color-fg-primary);
    text-align: start;
    cursor: pointer;
  }
  .wo-match:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .wo-match-time {
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .wo-empty {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .wo-list {
    display: flex;
    flex-direction: column;
  }
  .wo-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto 1.75rem;
    align-items: baseline;
    column-gap: 0.75rem;
    padding: 0.7rem 0;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }
  .wo-row:last-child {
    border-bottom: none;
  }
  .wo-name {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--color-fg-primary);
  }
  .wo-offset {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .wo-time {
    justify-self: end;
    white-space: nowrap;
    font-size: var(--text-xl);
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    color: var(--color-fg-primary);
  }
  .wo-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    align-self: center;
    width: 1.75rem;
    height: 1.75rem;
    border: none;
    border-radius: var(--radius-button, 6px);
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    cursor: pointer;
  }
  .wo-remove:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    color: var(--color-fg-primary);
  }
</style>
