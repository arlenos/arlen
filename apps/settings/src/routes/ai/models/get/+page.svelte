<script lang="ts">
  /// Get a model (model-catalog-and-picker.md, the acquisition sub-page): search
  /// first-class on top; at rest the surface shows exactly the three picks for
  /// this machine (instant, from the pre-computed ranking); typing turns the same
  /// surface into results, with the Hugging Face reach as an escalation of the
  /// same term. One kit-Row pattern, prose meta instead of badge rows; the only
  /// badges are "No safety guardrails" and Installed.
  import { onMount } from "svelte";
  import { ExternalLink, ShieldOff } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { Progress } from "@arlen/ui-kit/components/ui/progress";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { Input } from "@arlen/ui-kit/components/ui/input";
  import { t } from "$lib/i18n/messages";
  import {
    models,
    download,
    modelsMocked,
    hfSearch,
    uncensoredConfirmSeen,
    tierPicks,
    tierMeta,
    taskLabel,
    loadModels,
    startDownload,
    cancelDownload,
    searchHuggingFace,
    type Tier,
    type Model,
  } from "$lib/stores/models";

  onMount(loadModels);

  const TIERS: Tier[] = ["fast", "balanced", "quality"];
  const picks = $derived(tierPicks($models));

  let query = $state("");
  const searching = $derived(query.trim().length > 0);
  const results = $derived(
    $models
      .filter((m) => m.kind === "local" && !m.installed)
      .filter((m) => m.name.toLowerCase().includes(query.trim().toLowerCase()))
      .sort((a, b) => (a.paramsB ?? 0) - (b.paramsB ?? 0)),
  );
  // The rest of the catalogue, browsable without typing: everything local that is
  // neither installed nor already one of the three picks.
  const more = $derived.by(() => {
    const picked = new Set(TIERS.map((t) => picks[t]?.id).filter(Boolean));
    return $models
      .filter((m) => m.kind === "local" && !m.installed && !picked.has(m.id))
      .sort((a, b) => (a.paramsB ?? 0) - (b.paramsB ?? 0));
  });
  const offline = $derived($hfSearch !== null && !$hfSearch.reachable);

  // The download affirmation; the first no-guardrails download carries the honest
  // copy (refusal removed, lower quality), once ever.
  let pending = $state<Model | null>(null);
  const pendingFirstUncensored = $derived(pending?.uncensored === true && !$uncensoredConfirmSeen);
  async function confirmDownload() {
    const m = pending;
    pending = null;
    if (!m) return;
    if (m.uncensored) uncensoredConfirmSeen.set(true);
    await startDownload(m);
  }

  // One quiet prose line per model; the normal case (fits) stays silent.
  function fitPhrase(m: Model): string | null {
    if (m.fit === "wont-fit") return $t("s.mdl.fit.wontRun");
    if (m.fit === "may-be-slow") return $t("s.mdl.fit.slow");
    return null;
  }
  function resultMeta(m: Model): string {
    const parts: string[] = [];
    if (m.sizeGb != null) parts.push(`${m.sizeGb.toFixed(1)} GB`);
    const fit = fitPhrase(m);
    if (fit) parts.push(fit);
    return parts.join(" · ");
  }
  function pickMeta(m: Model, tier: Tier): string {
    const parts: string[] = [tierMeta(tier).label];
    if (m.sizeGb != null) parts.push(`${m.sizeGb.toFixed(1)} GB`);
    const fit = fitPhrase(m);
    if (fit) parts.push(fit);
    return parts.join(" · ");
  }

  function downloadPct(id: string): number | null {
    const d = $download;
    if (!d || d.id !== id) return null;
    return d.status === "verifying" ? 100 : (d.bytesFetched / d.totalBytes) * 100;
  }
</script>

<Page
  title={$t("s.mdl.get")}
  description={$t("s.mdl.get.desc")}
>
  <SectionGrid>
    {#if $modelsMocked}
      <p class="sample span-full">{$t("s.mdl.sample")}</p>
    {/if}

    <div class="search span-full">
      <Input bind:value={query} placeholder={$t("s.mdl.search")} aria-label={$t("s.mdl.search")} id="model-search" />
      {#if offline}
        <p class="quiet-note offline-note">{$t("s.mdl.offline")}</p>
      {/if}
    </div>

    {#if !searching}
      <Group label={$t("s.mdl.forMachine")} class="span-full">
        {#each TIERS as tier (tier)}
          {@const m = picks[tier]}
          {#if m}
            <Row label={m.name} description={pickMeta(m, tier)} id={`pick-${tier}`}>
              {#snippet control()}
                {@render action(m)}
              {/snippet}
              {#snippet below()}
                {@render tags(m)}
              {/snippet}
            </Row>
          {/if}
        {/each}
      </Group>

      {#if more.length > 0}
        <Group label={$t("s.mdl.more")} class="span-full">
          {#each more as m (m.id)}
            <Row label={m.name} description={resultMeta(m)} id={`more-${m.id}`}>
              {#snippet control()}
                {@render action(m)}
              {/snippet}
              {#snippet below()}
                {@render tags(m)}
              {/snippet}
            </Row>
          {/each}
        </Group>
      {/if}
    {:else}
      <Group label={$t("s.mdl.results")} class="span-full">
        {#each results as m (m.id)}
          <Row label={m.name} description={resultMeta(m)} id={`result-${m.id}`}>
            {#snippet control()}
              {@render action(m)}
            {/snippet}
            {#snippet below()}
              {@render tags(m)}
            {/snippet}
          </Row>
        {:else}
          <p class="quiet-note">{$t("s.mdl.noMatch")}</p>
        {/each}

        {#if !offline}
          <Button
            variant="ghost"
            class="w-full justify-start gap-2 px-4 font-normal text-muted-foreground hover:text-foreground"
            onclick={() => searchHuggingFace(query)}
          >
            <ExternalLink size={15} strokeWidth={1.75} />
            {$t("s.mdl.searchHfFor", { q: query })}
          </Button>
        {/if}
      </Group>
    {/if}
  </SectionGrid>
</Page>

<!-- What the model is for, as quiet tags under the row. -->
{#snippet tags(m: Model)}
  <span class="tags">
    {#each m.tasks as task (task)}<Badge variant="outline">{taskLabel(task)}</Badge>{/each}
  </span>
{/snippet}

<!-- The one action slot: download / progress+cancel / installed, plus the
     guardrails badge when the model has none. -->
{#snippet action(m: Model)}
  {@const pct = downloadPct(m.id)}
  <span class="row-control">
    {#if m.uncensored}
      <Badge variant="outline"><ShieldOff strokeWidth={2} />{$t("s.mdl.unc.badge")}</Badge>
    {/if}
    {#if pct !== null}
      <span class="dl">
        <Progress value={pct} />
        <span class="dl-row">
          <span class="dl-note">{$download?.status === "verifying" ? $t("s.mdl.verifying") : `${Math.round(pct)}%`}</span>
          <Button
            variant="link"
            size="sm"
            class="h-auto p-0 text-xs text-muted-foreground hover:text-destructive"
            onclick={() => cancelDownload(m.id)}
          >
            {$t("s.mdl.cancel")}
          </Button>
        </span>
      </span>
    {:else if m.installed}
      <!-- Same box as the sm button so the right column lines up across rows. -->
      <Badge variant="success" class="h-control px-2.5">{$t("s.mdl.installed")}</Badge>
    {:else}
      <Button
        variant="outline"
        size="sm"
        disabled={m.fit === "wont-fit" || $download !== null}
        onclick={() => (pending = m)}
      >
        {$t("s.mdl.download")}
      </Button>
    {/if}
  </span>
{/snippet}

<!-- The download affirmation. The first no-guardrails download carries the honest
     copy (refusal removed, lower quality), once ever; otherwise the plain
     size/egress confirm. -->
<ConfirmDialog
  open={pending !== null}
  title={pendingFirstUncensored ? $t("s.mdl.unc.confirmTitle") : $t("s.mdl.confirmTitle")}
  message={pending
    ? pendingFirstUncensored
      ? $t("s.mdl.unc.confirmMsg", { name: pending.name, size: pending.sizeGb?.toFixed(1) ?? "?" })
      : $t("s.mdl.confirmMsg", { name: pending.name, size: pending.sizeGb?.toFixed(1) ?? "?" })
    : ""}
  confirmLabel={$t("s.mdl.download")}
  onConfirm={confirmDownload}
  onCancel={() => (pending = null)}
/>

<style>
  .sample {
    margin: 0;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .search {
    padding: 0 0.25rem;
  }
  /* Pull the tag line toward its label block (the Row's main/below gap reads
     looser than the title/description gap). */
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 0.375rem;
    margin-top: -0.25rem;
  }
  .offline-note {
    padding: 0.4rem 0.25rem 0;
  }
  .quiet-note {
    margin: 0;
    padding: 0.5rem 1rem;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  /* Badge and action stack (end-aligned) so a badged row never pushes its
     button past the card edge. */
  .row-control {
    display: inline-flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 0.35rem;
  }
  .dl {
    display: inline-flex;
    flex-direction: column;
    gap: 0.25rem;
    width: 9rem;
  }
  .dl-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .dl-note {
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
</style>
