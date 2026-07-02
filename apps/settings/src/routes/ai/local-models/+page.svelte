<script lang="ts">
  /// Model Manager (local-model-bundle-plan.md): the curated, non-technical
  /// place to ACQUIRE and MANAGE local AI models. Three hardware-picked tiers
  /// (Fast / Balanced / Quality), a plain fit verdict computed locally, quant
  /// hidden, size in GB, download progress + cancel, delete to reclaim space.
  /// Choosing which model actually answers lives on the Default models page and
  /// the in-chat picker, not here (role split, Tim 2 July). Downloading is the
  /// single deliberate egress, so it is a clear one-time affirmation.
  ///
  /// The `ai-model-manager` backend already computes fit/speed/quant and does the
  /// verified download; the Settings Tauri bridge is unwired, so this reads a
  /// fixture and simulates progress. Affordance-only until the bridge lands.
  import { onMount } from "svelte";
  import { HardDrive, Trash2, Check, ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { LinkCard } from "@arlen/ui-kit/components/ui/link-card";
  import { Progress } from "@arlen/ui-kit/components/ui/progress";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import { SlidersHorizontal } from "lucide-svelte";
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

  const FIT: Record<Fit, { text: string; tone: "success" | "warn" | "destructive" }> = {
    fits: { text: "Fits", tone: "success" },
    "may-be-slow": { text: "May be slow", tone: "warn" },
    "wont-fit": { text: "Won't fit", tone: "destructive" },
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
        {#each installed as m (m.source)}
          <Row
            label={m.name}
            description={`${m.sizeGb.toFixed(1)} GB${m.baked ? " · built in" : ""}`}
            id={`local-${m.source}`}
          >
            {#snippet control()}
              <IconAction
                label={m.baked ? "The built-in model cannot be removed" : `Delete ${m.name}`}
                disabled={m.baked}
                onclick={() => deleteModel(m.source)}
              >
                <Trash2 size={15} strokeWidth={1.75} />
              </IconAction>
            {/snippet}
          </Row>
        {/each}
      </Group>
    {/if}

    <LinkCard
      href="/ai/models"
      title="Default models"
      description="Choose which model answers each kind of task"
    >
      {#snippet icon()}<SlidersHorizontal size={20} strokeWidth={1.75} />{/snippet}
    </LinkCard>

    {#if advanced.length > 0}
      <Collapsible class="adv span-full">
        <CollapsibleTrigger class="adv-trigger">
          <ChevronRight size={15} strokeWidth={2} />
          Advanced
        </CollapsibleTrigger>
        <CollapsibleContent>
          <Group class="span-full">
            <p class="adv-note">Uncurated models from the wider community. No guarantees on quality or safety.</p>
            {#each advanced as m (m.source)}
              <div class="adv-model">{@render modelBody(m)}</div>
            {/each}
          </Group>
        </CollapsibleContent>
      </Collapsible>
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
        {#each m.tasks as t (t)}<Badge variant="outline">{taskLabel(t)}</Badge>{/each}
        <Badge variant={FIT[m.fit].tone}>{FIT[m.fit].text}</Badge>
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
    padding: 0.25rem 1rem;
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
  .model-meta {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
  }
  .model-action {
    flex-shrink: 0;
  }
  .installed {
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

  /* The Advanced disclosure trigger + its rotating chevron. The class rides the
     Collapsible component root, so these are fully global. */
  :global(.adv-trigger) {
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
  :global(.adv-trigger:hover) {
    color: var(--foreground);
  }
  :global(.adv-trigger svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  :global(.adv-trigger[data-state="open"] svg) {
    transform: rotate(90deg);
  }
  .adv-note {
    margin: 0;
    padding: var(--space-row, 0.75rem) 1rem 0.25rem;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .adv-model {
    padding: 0.375rem 1rem;
  }
</style>
