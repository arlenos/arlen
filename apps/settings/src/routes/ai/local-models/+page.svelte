<script lang="ts">
  /// Model Manager (local-model-bundle-plan.md): the curated, non-technical
  /// place to download and manage local AI models. Three hardware-picked tiers
  /// (Fast / Balanced / Quality), a plain fit verdict computed locally, quant
  /// hidden, size in GB, download progress + cancel, delete to reclaim space,
  /// one active model. Downloading is the single deliberate egress a no-telemetry
  /// OS makes, so it is a clear one-time affirmation, never a hidden dependency.
  ///
  /// The `ai-model-manager` backend already computes fit/speed/quant and does the
  /// verified download; the Settings Tauri bridge is unwired, so this reads a
  /// fixture and simulates progress. Affordance-only until the bridge lands.
  import { onMount } from "svelte";
  import { HardDrive, Trash2, Check, ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Progress } from "@arlen/ui-kit/components/ui/progress";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    models,
    hardware,
    download,
    modelsLoaded,
    tierPicks,
    installedModels,
    advancedModels,
    tierMeta,
    taskLabel,
    loadLocalModels,
    startDownload,
    cancelDownload,
    setActive,
    deleteModel,
    type Tier,
    type LocalModel,
    type Fit,
  } from "$lib/stores/localModels";

  onMount(loadLocalModels);

  const picks = $derived(tierPicks($models));
  const TIERS: Tier[] = ["fast", "balanced", "quality"];
  const installed = $derived(installedModels($models));
  const advanced = $derived(advancedModels($models));

  let showAdvanced = $state(false);

  // The one consented egress: a clear one-line affirmation before a download.
  let pending = $state<LocalModel | null>(null);
  function askDownload(m: LocalModel) {
    pending = m;
  }
  async function confirmDownload() {
    const m = pending;
    pending = null;
    if (m) await startDownload(m);
  }

  const FIT_TEXT: Record<Fit, string> = {
    fits: "Fits",
    "may-be-slow": "May be slow",
    "wont-fit": "Won't fit",
  };

  function downloadPct(source: string): number | null {
    const d = $download;
    if (!d || d.source !== source) return null;
    return d.status === "verifying" ? 100 : (d.bytesFetched / d.totalBytes) * 100;
  }
</script>

<Page
  title="Local models"
  description="Download and manage AI models that run on your own machine, offline. Nothing here leaves your computer except a model download, which you confirm each time."
>
  <SectionGrid>
    {#if $hardware}
      <div class="hw span-full">
        <HardDrive size={15} strokeWidth={1.75} />
        <span>{$hardware.summary}</span>
      </div>
    {/if}

    <Group label="Recommended" class="span-full">
      <div class="tiers">
        {#each TIERS as tier (tier)}
          {@const m = picks[tier]}
          {@const meta = tierMeta(tier)}
          <div class="tier">
            <div class="tier-head">
              <span class="tier-label">{meta.label}</span>
              <span class="tier-note">{meta.note}</span>
            </div>
            {#if m}
              {@render modelBody(m)}
            {:else}
              <p class="tier-empty">Nothing in this tier runs well on your machine.</p>
            {/if}
          </div>
        {/each}
      </div>
    </Group>

    {#if installed.length > 0}
      <Group label="Your models" class="span-full">
        {#each installed as m, i (m.source)}
          {#if i > 0}<div class="hr"></div>{/if}
          <div class="mine">
            <div class="mine-info">
              <span class="mine-name">{m.name}</span>
              <span class="mine-meta">
                {m.sizeGb.toFixed(1)} GB{m.baked ? " · built in" : ""}
              </span>
            </div>
            {#if m.active}
              <span class="active"><Check size={13} strokeWidth={2.5} /> Active</span>
            {:else}
              <Button variant="outline" size="sm" onclick={() => setActive(m.source)}>Use</Button>
            {/if}
            <button
              type="button"
              class="del"
              aria-label={`Delete ${m.name}`}
              disabled={m.baked}
              title={m.baked ? "The built-in model cannot be removed" : "Delete to reclaim space"}
              onclick={() => deleteModel(m.source)}
            >
              <Trash2 size={15} strokeWidth={1.75} />
            </button>
          </div>
        {/each}
      </Group>
    {/if}

    {#if advanced.length > 0}
      <div class="span-full">
        <button
          type="button"
          class="adv-toggle"
          class:open={showAdvanced}
          onclick={() => (showAdvanced = !showAdvanced)}
        >
          <ChevronRight size={15} strokeWidth={2} />
          Advanced
        </button>
        {#if showAdvanced}
          <Group class="span-full">
            <p class="adv-note">Uncurated models from the wider community. No guarantees on quality or safety.</p>
            {#each advanced as m, i (m.source)}
              {#if i > 0}<div class="hr"></div>{/if}
              <div class="adv-model">{@render modelBody(m)}</div>
            {/each}
          </Group>
        {/if}
      </div>
    {/if}

    {#if $modelsLoaded && $models.length === 0}
      <Group label="Local models" class="span-full">
        <p class="tier-empty">No models are available.</p>
      </Group>
    {/if}
  </SectionGrid>
</Page>

<!-- A model's name, tags, fit, size, and the right action (download / progress /
     installed), shared by the tier cards and the advanced list. -->
{#snippet modelBody(m: LocalModel)}
  {@const pct = downloadPct(m.source)}
  <div class="model">
    <div class="model-info">
      <span class="model-name">{m.name}</span>
      <span class="model-tags">
        {#each m.tasks as t (t)}<span class="tag">{taskLabel(t)}</span>{/each}
        <span class="fit fit-{m.fit}">{FIT_TEXT[m.fit]}</span>
      </span>
      <span class="model-meta">{m.sizeGb.toFixed(1)} GB · {Math.round(m.tokensPerSec)} words/sec</span>
    </div>
    <div class="model-action">
      {#if pct !== null}
        <div class="dl">
          <Progress value={pct} />
          <div class="dl-row">
            <span class="dl-status">{$download?.status === "verifying" ? "Verifying…" : `${Math.round(pct)}%`}</span>
            <button type="button" class="dl-cancel" onclick={() => cancelDownload(m.source)}>Cancel</button>
          </div>
        </div>
      {:else if m.installed}
        <span class="installed"><Check size={13} strokeWidth={2.5} /> Installed</span>
      {:else}
        <Button
          variant={m.fit === "wont-fit" ? "outline" : "default"}
          size="sm"
          disabled={m.fit === "wont-fit" || $download !== null}
          onclick={() => askDownload(m)}
        >
          Download
        </Button>
      {/if}
    </div>
  </div>
{/snippet}

<ConfirmDialog
  open={pending !== null}
  title="Download this model?"
  message={pending
    ? `This downloads ${pending.name} (${pending.sizeGb.toFixed(1)} GB) from Hugging Face. It is the one time Arlen reaches out; after that the model runs fully offline.`
    : ""}
  confirmLabel="Download"
  onConfirm={confirmDownload}
  onCancel={() => (pending = null)}
/>

<style>
  .hw {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0 0.25rem;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }

  .tiers {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0;
  }
  .tier {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
    padding: 1rem;
    border-right: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .tier:last-child {
    border-right: none;
  }
  .tier-head {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }
  .tier-label {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--foreground);
  }
  .tier-note {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .tier-empty {
    margin: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  /* One model: info on the left, the action on the right. */
  .model {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 0.75rem;
  }
  .model-info {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    min-width: 0;
  }
  .model-name {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .model-tags {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.375rem;
  }
  .tag {
    font-size: 0.625rem;
    padding: 0.0625rem 0.375rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }
  .fit {
    font-size: 0.625rem;
    font-weight: 600;
    padding: 0.0625rem 0.375rem;
    border-radius: var(--radius-chip, 4px);
  }
  .fit-fits {
    color: var(--color-success, #16a34a);
    background: color-mix(in srgb, var(--color-success, #16a34a) 14%, transparent);
  }
  .fit-may-be-slow {
    color: var(--color-warning, #ca8a04);
    background: color-mix(in srgb, var(--color-warning, #ca8a04) 14%, transparent);
  }
  .fit-wont-fit {
    color: var(--color-error, #dc2626);
    background: color-mix(in srgb, var(--color-error, #dc2626) 14%, transparent);
  }
  .model-meta {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .model-action {
    flex-shrink: 0;
  }
  .installed,
  .active {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--color-success, #16a34a);
  }

  .dl {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    width: 9rem;
  }
  .dl-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .dl-status {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .dl-cancel {
    border: none;
    background: transparent;
    padding: 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    cursor: pointer;
  }
  .dl-cancel:hover {
    color: var(--color-error, #dc2626);
  }

  /* Your models rows. */
  .mine {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 0;
  }
  .mine-info {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .mine-name {
    font-size: 0.8125rem;
    font-weight: 500;
    color: var(--foreground);
  }
  .mine-meta {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .del {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-button, 6px);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    cursor: pointer;
    transition: color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .del:hover:not(:disabled) {
    color: var(--color-error, #dc2626);
  }
  .del:disabled {
    opacity: 0.35;
    cursor: default;
  }

  .hr {
    height: 1px;
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }

  .adv-toggle {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.5rem 0.25rem;
    border: none;
    background: transparent;
    font-size: 0.8125rem;
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
  }
  .adv-toggle:hover {
    color: var(--foreground);
  }
  .adv-toggle :global(svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .adv-toggle.open :global(svg) {
    transform: rotate(90deg);
  }
  .adv-note {
    margin: 0 0 0.25rem;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .adv-model {
    padding: 0.25rem 0;
  }
</style>
