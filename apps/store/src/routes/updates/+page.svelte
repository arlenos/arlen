<script lang="ts">
  /// Pending updates (update-flow-plan.md U-5): grouped by consequence, never
  /// alphabet. A capability widening needs your decision and sits on top with
  /// the delta as a first-class line; routine updates sit below with honest
  /// release notes (the upstream text or "the developer didn't say"), never a
  /// fabricated changelog. Quiet by default so the loud case is believed.
  import { onMount } from "svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import StoreHeader from "$lib/components/StoreHeader.svelte";
  import StoreRail from "$lib/components/StoreRail.svelte";
  import { t } from "$lib/i18n/messages";
  import {
    pendingUpdates,
    updatesMocked,
    loadUpdates,
    applyUpdate,
    applyAllRoutine,
    skipUpdate,
    type PendingUpdate,
  } from "$lib/stores/updates";

  onMount(loadUpdates);

  const widened = $derived($pendingUpdates.filter((u) => u.delta === "widened"));
  const routine = $derived($pendingUpdates.filter((u) => u.delta !== "widened"));

  function sourceLabel(s: string): string {
    return s === "forage" ? $t("st.src.forage") : s === "flathub" ? $t("st.src.flathub") : $t("st.src.debian");
  }
</script>

<div class="st-app">
  <StoreHeader />
  <div class="st-body">
    <StoreRail />
    <main class="st-main">
      <div class="st-content">
        {#if $updatesMocked}
          <p class="sample">{$t("st.sample")}</p>
        {/if}

        {#if $pendingUpdates.length === 0}
          <p class="quiet">{$t("st.upd.empty")}</p>
        {:else}
          {#if widened.length > 0}
            <div class="group-label">{$t("st.upd.decision")}</div>
            {#each widened as u (u.id)}
              <div class="upd" id={`upd-${u.id}`}>
                <span class="tile" style="background:{u.icon}" aria-hidden="true"></span>
                <div class="upd-body">
                  <div class="upd-head">
                    <span class="upd-name">{u.name}</span>
                    <span class="upd-ver">{u.from} &rarr; {u.to}, {sourceLabel(u.source)}</span>
                  </div>
                  {#each u.deltaLines as line (line)}
                    <p class="delta">{line}</p>
                  {/each}
                  {#if u.notes}
                    <p class="notes">{u.notes}</p>
                  {/if}
                  <div class="actions">
                    <Button size="sm" onclick={() => applyUpdate(u.id)}>{$t("st.upd.update")}</Button>
                    <Button variant="ghost" size="sm" onclick={() => skipUpdate(u.id)}>{$t("st.upd.skip")}</Button>
                    <Button variant="ghost" size="sm" class="text-muted-foreground">{$t("st.upd.uninstall")}</Button>
                  </div>
                </div>
              </div>
            {/each}
          {/if}

          {#if routine.length > 0}
            <div class="group-row">
              <div class="group-label routine-label">{$t("st.upd.routine")}</div>
              <Button variant="outline" size="sm" id="update-all-routine" onclick={() => applyAllRoutine()}>
                {$t("st.upd.updateAll")}
              </Button>
            </div>
            {#each routine as u (u.id)}
              <div class="upd" id={`upd-${u.id}`}>
                <span class="tile" style="background:{u.icon}" aria-hidden="true"></span>
                <div class="upd-body">
                  <div class="upd-head">
                    <span class="upd-name">{u.name}</span>
                    <span class="upd-ver">{u.from} &rarr; {u.to}, {sourceLabel(u.source)}</span>
                  </div>
                  {#each u.deltaLines as line (line)}
                    <p class="delta narrowed">{line}</p>
                  {/each}
                  <p class="notes">{u.notes ?? $t("st.upd.noNotes")}</p>
                  <div class="actions">
                    <Button variant="outline" size="sm" onclick={() => applyUpdate(u.id)}>{$t("st.upd.update")}</Button>
                  </div>
                </div>
              </div>
            {/each}
          {/if}
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
  .quiet {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .group-label {
    margin: 1.25rem 0 0.6rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .group-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .routine-label {
    margin-bottom: 0;
  }
  .group-row {
    margin: 1.25rem 0 0.6rem;
  }

  .upd {
    display: flex;
    gap: 0.75rem;
    align-items: flex-start;
    margin-bottom: 0.6rem;
    padding: 0.75rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-primary) 2%, transparent);
  }
  .tile {
    flex-shrink: 0;
    width: 2.5rem;
    height: 2.5rem;
    border-radius: var(--radius-input);
  }
  .upd-body {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .upd-head {
    display: flex;
    align-items: baseline;
    gap: 0.6rem;
    flex-wrap: wrap;
  }
  .upd-name {
    font-size: var(--text-sm);
    font-weight: 600;
  }
  .upd-ver {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
    font-variant-numeric: tabular-nums;
  }
  /* The capability delta is the first-class line: full foreground when it
     widens (the one loud case), quiet when it narrows. */
  .delta {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.45;
    color: var(--color-fg-primary);
  }
  .delta.narrowed {
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .notes {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 52%, transparent);
  }
  .actions {
    display: flex;
    gap: 0.4rem;
    padding-top: 0.25rem;
  }
</style>
