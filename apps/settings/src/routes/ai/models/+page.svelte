<script lang="ts">
  /// The Models hub (merges the old Default-models picker + the Model Manager).
  /// One surface to: assign which model answers each task, get a curated model
  /// for this machine, browse/search for a specific one, and import your own.
  /// Choosing lives here (per-role); Providers stays separate (connect accounts).
  ///
  /// Almost everything is a fixture today: the daemon stores one active model
  /// (per-role is new backend), cannot enumerate downloaded models, has no HF
  /// search and no import. The store reads the intended commands, then mocks.
  import { onMount } from "svelte";
  import { HardDrive, Trash2, Check, Search, Upload, ExternalLink } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { Badge } from "@arlen/ui-kit/components/ui/badge";
  import { Progress } from "@arlen/ui-kit/components/ui/progress";
  import { ConfirmDialog } from "@arlen/ui-kit/components/ui/confirm-dialog";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { ProviderLogo } from "@arlen/ui-kit/components/ui/provider-logo";
  import {
    models,
    hardware,
    download,
    modelsLoaded,
    roles,
    hfSearch,
    installedModels,
    availableModels,
    tierPicks,
    tierMeta,
    roleMeta,
    taskLabel,
    modelById,
    loadModels,
    setRole,
    startDownload,
    cancelDownload,
    deleteModel,
    importModel,
    searchHuggingFace,
    type Role,
    type Tier,
    type Model,
    type Fit,
  } from "$lib/stores/models";

  onMount(loadModels);

  const ROLES: Role[] = ["query", "agent", "title"];
  const TIERS: Tier[] = ["fast", "balanced", "quality"];
  const picks = $derived(tierPicks($models));

  const roleOptions = $derived($availableModels.map((m) => ({ value: m.id, label: m.name })));

  // Browse: search + task filter over the curated local list, uncurated hidden.
  let query = $state("");
  let taskFilter = $state("all");
  let showAdvanced = $state(false);
  const TASK_OPTIONS = [
    { value: "all", label: "All tasks" },
    { value: "general", label: "Everyday" },
    { value: "coding", label: "Coding" },
    { value: "reasoning", label: "Reasoning" },
    { value: "writing", label: "Writing" },
  ];
  const browseList = $derived(
    $models
      .filter((m) => m.kind === "local")
      .filter((m) => showAdvanced || !m.advanced)
      .filter((m) => taskFilter === "all" || m.tasks.includes(taskFilter))
      .filter((m) => m.name.toLowerCase().includes(query.trim().toLowerCase()))
      .sort((a, b) => (a.paramsB ?? 0) - (b.paramsB ?? 0)),
  );

  // The one consented egress: a clear affirmation before a download.
  let pending = $state<Model | null>(null);
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

  function downloadPct(id: string): number | null {
    const d = $download;
    if (!d || d.id !== id) return null;
    return d.status === "verifying" ? 100 : (d.bytesFetched / d.totalBytes) * 100;
  }

  function meta(m: Model): string {
    const parts: string[] = [];
    if (m.baked) parts.push("built in");
    if (m.imported) parts.push("imported");
    if (m.sizeGb != null) parts.push(`${m.sizeGb.toFixed(1)} GB`);
    return parts.join(" · ");
  }
</script>

<Page
  title="Models"
  description="Pick which model answers each task, get new ones made for your machine, or bring your own. Everything runs on your computer unless you connect a cloud service."
>
  <SectionGrid>
    {#if $hardware}
      <div class="hw span-full">
        <HardDrive size={15} strokeWidth={1.75} />
        <span>{$hardware.summary}</span>
      </div>
    {/if}

    <Group label="Active" class="span-full">
      {#each ROLES as role (role)}
        {@const rm = roleMeta(role)}
        <Row label={rm.label} description={rm.description} id={`role-${role}`}>
          {#snippet control()}
            <PopoverSelect
              value={$roles[role]}
              options={roleOptions}
              ariaLabel={`${rm.label} model`}
              width="15rem"
              onchange={(v) => setRole(role, v)}
              renderLabel={modelOption as never}
            />
          {/snippet}
        </Row>
      {/each}
    </Group>

    <Group label="Recommended for your machine" class="span-full">
      <div class="tiers">
        {#each TIERS as tier (tier)}
          {@const m = picks[tier]}
          {@const tm = tierMeta(tier)}
          <div class="tier">
            <div class="tier-head">
              <span class="tier-label">{tm.label}</span>
              <span class="tier-note">{tm.note}</span>
            </div>
            {#if m}
              {@render modelBody(m)}
            {:else}
              <p class="muted-line">Nothing in this tier runs well on your machine.</p>
            {/if}
          </div>
        {/each}
      </div>
    </Group>

    {#if $installedModels.length > 0}
      <Group label="Your models" class="span-full">
        {#each $installedModels as m (m.id)}
          <Row label={m.name} description={meta(m)} id={`installed-${m.id}`}>
            {#snippet control()}
              <IconAction
                label={m.baked ? "The built-in model cannot be removed" : `Delete ${m.name}`}
                disabled={m.baked}
                onclick={() => deleteModel(m.id)}
              >
                <Trash2 size={15} strokeWidth={1.75} />
              </IconAction>
            {/snippet}
          </Row>
        {/each}
        <button type="button" class="import-row" onclick={() => importModel()}>
          <Upload size={15} strokeWidth={1.75} />
          Import a model from your computer
        </button>
      </Group>
    {/if}

    <Group label="Browse more" class="span-full">
      <div class="browse-bar">
        <span class="search">
          <Search size={14} strokeWidth={2} />
          <input type="text" bind:value={query} placeholder="Search models" aria-label="Search models" />
        </span>
        <PopoverSelect
          value={taskFilter}
          options={TASK_OPTIONS}
          ariaLabel="Filter by task"
          width="11rem"
          onchange={(v) => (taskFilter = v)}
        />
        <button
          type="button"
          class="hf-search"
          onclick={() => searchHuggingFace()}
          title="Reach out to Hugging Face to widen the results"
        >
          Search Hugging Face
          <ExternalLink size={13} strokeWidth={2} />
        </button>
      </div>

      {#if $hfSearch}
        <p class="muted-line browse-note">Showing curated models plus results from Hugging Face.</p>
      {/if}

      {#each browseList as m (m.id)}
        <div class="browse-row">{@render modelBody(m)}</div>
      {:else}
        <p class="muted-line browse-note">No models match. Try a different search or Hugging Face.</p>
      {/each}

      <label class="adv-check">
        <input type="checkbox" bind:checked={showAdvanced} />
        Show uncurated community models
      </label>
    </Group>

    {#if $modelsLoaded && $models.length === 0}
      <Group label="Models" class="span-full">
        <p class="muted-line">No models are available.</p>
      </Group>
    {/if}
  </SectionGrid>
</Page>

<!-- The picker label: a local model shows the on-device mark, a cloud model its
     provider logo, then the name. Cast to `never` (kit vs app resolve `svelte`
     to distinct Snippet types, identical at runtime). -->
{#snippet modelOption(opt: { value: string; label: string })}
  {@const m = modelById($availableModels, opt.value)}
  <span class="opt">
    {#if m?.kind === "cloud"}
      <ProviderLogo id={m.provider} size={18} />
    {:else}
      <HardDrive size={16} strokeWidth={1.75} />
    {/if}
    <span class="opt-label">{opt.label}</span>
  </span>
{/snippet}

<!-- A model's name, tags, fit, size, and the right action (download / progress /
     installed), shared by the tier cards and the browse list. -->
{#snippet modelBody(m: Model)}
  {@const pct = downloadPct(m.id)}
  <div class="model">
    <div class="model-info">
      <span class="model-name">{m.name}</span>
      <span class="model-tags">
        {#each m.tasks as t (t)}<Badge variant="outline">{taskLabel(t)}</Badge>{/each}
        {#if m.fit}<Badge variant={FIT[m.fit].tone}>{FIT[m.fit].text}</Badge>{/if}
      </span>
      <span class="model-meta">
        {m.sizeGb != null ? `${m.sizeGb.toFixed(1)} GB` : ""}
        {#if m.tokensPerSec != null}· {Math.round(m.tokensPerSec)} words/sec{/if}
      </span>
    </div>
    <div class="model-action">
      {#if pct !== null}
        <div class="dl">
          <Progress value={pct} />
          <div class="dl-row">
            <span class="muted-line">{$download?.status === "verifying" ? "Verifying…" : `${Math.round(pct)}%`}</span>
            <button type="button" class="dl-cancel" onclick={() => cancelDownload(m.id)}>Cancel</button>
          </div>
        </div>
      {:else if m.installed}
        <span class="installed"><Check size={13} strokeWidth={2.5} /> Installed</span>
      {:else}
        <Button
          variant={m.fit === "wont-fit" ? "outline" : "default"}
          size="sm"
          disabled={m.fit === "wont-fit" || $download !== null}
          onclick={() => (pending = m)}
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
    ? `This downloads ${pending.name} (${pending.sizeGb?.toFixed(1)} GB) from Hugging Face. It is the one time Arlen reaches out; after that the model runs fully offline.`
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
  .muted-line {
    margin: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }

  /* Recommended tiers: three bordered columns inside one card. */
  .tiers {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
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

  /* One model row body: info left, action right. */
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

  /* Your models: the import affordance row. */
  .import-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.75rem 1rem;
    border: none;
    background: transparent;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    cursor: pointer;
    text-align: left;
  }
  .import-row:hover {
    color: var(--foreground);
  }

  /* Browse: the search + filter bar, the result rows, the advanced toggle. */
  .browse-bar {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    flex-wrap: wrap;
  }
  .search {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    flex: 1;
    min-width: 12rem;
    height: 2rem;
    padding: 0 0.625rem;
    border-radius: var(--radius-button, 6px);
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--foreground);
    font-size: 0.8125rem;
    outline: none;
  }
  .hf-search {
    display: inline-flex;
    align-items: center;
    gap: 0.375rem;
    height: 2rem;
    padding: 0 0.75rem;
    border: 1px solid color-mix(in srgb, var(--foreground) 14%, transparent);
    border-radius: var(--radius-button, 6px);
    background: transparent;
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    cursor: pointer;
  }
  .hf-search:hover {
    color: var(--foreground);
    border-color: color-mix(in srgb, var(--foreground) 30%, transparent);
  }
  .browse-note {
    padding: 0 1rem 0.25rem;
  }
  .browse-row {
    padding: 0.5rem 1rem;
  }
  .adv-check {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 1rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    cursor: pointer;
  }

  /* The picker label: logo/mark beside the model name. */
  .opt {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }
  .opt-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
