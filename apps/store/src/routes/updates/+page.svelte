<script lang="ts">
  /// Pending updates (update-flow-plan.md U-5): grouped by consequence, never
  /// alphabet. A capability widening needs your decision and sits on top with
  /// the delta as a first-class line - and an UNKNOWN delta sits there too,
  /// because "couldn't compare" is not routine. A widened row offers no Update
  /// button: installd's gate would refuse it, and nothing in this window can
  /// answer the consent it asks for yet, so the honest choices are Skip and
  /// Uninstall - said in the row, not implied by a button that silently fails.
  /// Every row carries its own state (updating, refused with the daemon's
  /// sentence, not started, unconfirmed); a skipped row moves down, never away.
  import { onMount } from "svelte";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { t } from "$lib/i18n/messages";
  import { capText } from "$lib/caps";
  import IconTile from "$lib/components/IconTile.svelte";
  import { loadCatalog, uninstallApp, uninstallStatus } from "$lib/stores/catalog";
  import {
    pendingUpdates,
    skippedUpdates,
    updatesMocked,
    rowStatus,
    loadUpdates,
    applyUpdate,
    applyAllRoutine,
    skipUpdate,
    forgetUpdate,
    deltaOf,
    updateApp,
    type PendingUpdate,
    type SourceLayer,
  } from "$lib/stores/updates";

  onMount(() => {
    void loadCatalog();
    void loadUpdates();
  });

  // Unknown joins the decision group: the one thing it must never be is quiet.
  const decision = $derived($pendingUpdates.filter((u) => deltaOf(u) !== "none"));
  const routine = $derived($pendingUpdates.filter((u) => deltaOf(u) === "none"));

  const LAYER_KEY: Record<SourceLayer, string> = {
    Personal: "st.src.forage",
    Community: "st.src.forage",
    Official: "st.src.forage",
    Flatpak: "st.src.flathub",
    Apt: "st.src.debian",
    Native: "st.src.native",
  };

  /// A row is busy while its update or its removal is in flight.
  function busy(id: string): boolean {
    const s = $rowStatus[id];
    return s?.kind === "applying" || $uninstallStatus[id]?.kind === "removing";
  }

  async function remove(u: PendingUpdate) {
    if (await uninstallApp(u.id)) forgetUpdate(u.id);
  }
</script>

<main class="st-main">
  <div class="st-content">
    {#if $updatesMocked}
      <p class="sample">{$t("st.sample")}</p>
    {/if}

    {#if $pendingUpdates.length === 0 && $skippedUpdates.length === 0}
      <p class="quiet">{$t("st.upd.empty")}</p>
    {:else}
      {#if decision.length > 0}
        <div class="group-label">{$t("st.upd.decision")}</div>
        {#each decision as u (u.id)}
          {@const app = updateApp(u.id)}
          {@const status = $rowStatus[u.id]}
          {@const removal = $uninstallStatus[u.id]}
          <div class="upd" id={`upd-${u.id}`}>
            <IconTile icon={app.icon} name={app.name} size="2.5rem" />
            <div class="upd-body">
              <div class="upd-head">
                <span class="upd-name">{app.name}</span>
                <span class="upd-ver">{u.installed_version} &rarr; {u.available_version}, {$t(LAYER_KEY[u.layer])}</span>
              </div>
              {#if u.new_capabilities === null}
                <p class="delta">{$t("st.upd.unknownDelta")}</p>
              {:else}
                {#each u.new_capabilities as cap (cap)}
                  <p class="delta">{$t("st.upd.wants", { what: capText($t, cap) })}</p>
                {/each}
                <!-- The system's own sentence about the one answer it cannot give
                     yet. A widened update has no Update button: the gate would
                     refuse it, and a button that fails in silence is worse than a
                     line that says so. -->
                <p class="cannot">{$t("st.upd.cannotAllow")}</p>
              {/if}
              {@render state(u.id, status, removal)}
              <div class="actions">
                {#if u.new_capabilities === null}
                  <Button size="sm" disabled={busy(u.id)} onclick={() => applyUpdate(u.id)}>{$t("st.upd.update")}</Button>
                {/if}
                <Button variant="ghost" size="sm" disabled={busy(u.id)} onclick={() => skipUpdate(u.id)}>{$t("st.upd.skip")}</Button>
                <Button variant="ghost" size="sm" class="text-muted-foreground" disabled={busy(u.id)} onclick={() => remove(u)}>
                  {$t("st.upd.uninstall")}
                </Button>
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
          {@const app = updateApp(u.id)}
          {@const status = $rowStatus[u.id]}
          <div class="upd" id={`upd-${u.id}`}>
            <IconTile icon={app.icon} name={app.name} size="2.5rem" />
            <div class="upd-body">
              <div class="upd-head">
                <span class="upd-name">{app.name}</span>
                <span class="upd-ver">{u.installed_version} &rarr; {u.available_version}, {$t(LAYER_KEY[u.layer])}</span>
              </div>
              {@render state(u.id, status, undefined)}
              <div class="actions">
                <Button variant="outline" size="sm" disabled={busy(u.id)} onclick={() => applyUpdate(u.id)}>
                  {status?.kind === "refused" ? $t("st.upd.retry") : $t("st.upd.update")}
                </Button>
              </div>
            </div>
          </div>
        {/each}
      {/if}

      {#if $skippedUpdates.length > 0}
        <!-- Skipped stays on the page, quiet: the user decided, and a capability
             widening that vanished is the one thing they must be able to
             revisit (U-4). -->
        <div class="group-label">{$t("st.upd.skipped")}</div>
        {#each $skippedUpdates as u (u.id)}
          {@const app = updateApp(u.id)}
          <div class="upd skipped" id={`upd-${u.id}`}>
            <IconTile icon={app.icon} name={app.name} size="2.5rem" />
            <div class="upd-body">
              <div class="upd-head">
                <span class="upd-name">{app.name}</span>
                <span class="upd-ver">{u.installed_version} &rarr; {u.available_version}, {$t(LAYER_KEY[u.layer])}</span>
              </div>
              <p class="quiet-line">{$t("st.upd.skippedHint")}</p>
            </div>
          </div>
        {/each}
      {/if}
    {/if}
  </div>
</main>

<!-- One row's state line: what its update or removal is doing right now, in
     the row it is about. -->
{#snippet state(
  id: string,
  status: { kind: string; reason?: string } | undefined,
  removal: { kind: string; reason?: string } | undefined,
)}
  {#if removal?.kind === "removing"}
    <p class="quiet-line">{$t("st.app.removing")}</p>
  {:else if removal?.kind === "refused"}
    <p class="refused" role="alert">{$t("st.app.uninstallRefused", { reason: removal.reason ?? "" })}</p>
  {:else if status?.kind === "applying"}
    <p class="quiet-line">{$t("st.upd.applying")}</p>
  {:else if status?.kind === "unconfirmed"}
    <p class="quiet-line">{$t("st.upd.unconfirmed")}</p>
  {:else if status?.kind === "notStarted"}
    <p class="quiet-line">{$t("st.upd.notStarted")}</p>
  {:else if status?.kind === "refused"}
    <p class="refused" role="alert">{$t("st.upd.refused", { reason: status.reason ?? "" })}</p>
  {/if}
{/snippet}

<style>
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
    margin: 1.25rem 0 0.6rem;
  }
  .routine-label {
    margin-bottom: 0;
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
  /* A skipped row is the same card at rest: read, decided, kept. */
  .upd.skipped {
    opacity: 0.6;
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
  /* The capability delta is the first-class line: full foreground, the one
     loud case on the page. */
  .delta {
    margin: 0;
    font-size: var(--text-sm);
    line-height: 1.45;
    color: var(--color-fg-primary);
  }
  /* The system's sentence under a widening: quieter than the delta, still a
     full sentence, never a badge. */
  .cannot,
  .quiet-line {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .refused {
    margin: 0;
    font-size: var(--text-xs);
    line-height: 1.45;
    color: var(--color-error, #dc2626);
  }
  .actions {
    display: flex;
    gap: 0.4rem;
    padding-top: 0.25rem;
  }
</style>
