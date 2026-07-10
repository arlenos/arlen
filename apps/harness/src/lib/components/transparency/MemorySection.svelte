<script lang="ts">
  /// Memory: the shape of what the AI holds right now, never the content.
  /// Entity types and node counts and the active task's declared reads,
  /// from the ai_working_set endpoint. Rendering the held content would
  /// itself be the Recall failure (the user's own data surfaced twice),
  /// so this is shape only by contract. Rendering only.
  import { t } from "$lib/i18n/messages";
  import { StatGrid } from "@arlen/ui-kit/components/ui/stat-grid";
  import { StatTile } from "@arlen/ui-kit/components/ui/stat-grid";
  import type { Capability } from "$lib/capability";
  import { reachLabel, readsSentence, type WorkingSet } from "$lib/transparency";
  import SectionState from "./SectionState.svelte";

  let {
    workingSet,
    capability,
    loaded,
  }: {
    workingSet: WorkingSet | null;
    capability: Capability | null;
    loaded: boolean;
  } = $props();

  const off = $derived(capability !== null && !capability.enabled);

  // The whole task sentence is built here so the optional "(reads ...)"
  // clause keeps its leading space (Svelte trims text around {#if}).
  const taskLine = $derived.by(() => {
    if (!workingSet?.activeBehaviour) return null;
    const phrase = readsSentence(workingSet.declaredReads);
    const reads = phrase ? ` (reads ${phrase})` : "";
    return `Active task: ${workingSet.activeBehaviour}${reads}.`;
  });
</script>

{#if off}
  <SectionState
    tag="AI is off"
    tone="off"
    message={$t("h.memory.off")}
  />
{:else if !loaded}
  <SectionState message={$t("h.memory.checking")} />
{:else if workingSet === null}
  <SectionState message={$t("h.memory.cantRead")} />
{:else if !workingSet.available}
  <SectionState
    tag={$t("h.memory.notMeasuredTitle")}
    tone="info"
    message={$t("h.memory.notMeasured")}
  />
{:else if !workingSet.held}
  <SectionState message={$t("h.memory.none")} />
{:else}
  <div class="memory">
    <div class="tiles">
      <StatGrid>
        {#each workingSet.entityCounts as c (c.type)}
          <StatTile label={reachLabel(c.type)} value={`${c.count}`} />
        {/each}
      </StatGrid>
    </div>
    {#if taskLine}
      <p class="task">{taskLine}</p>
    {/if}
    <p class="shape">
      This is the shape of what it holds, never the contents of your files.
    </p>
  </div>
{/if}

<style>
  .memory {
    display: flex;
    flex-direction: column;
    gap: 0.625rem;
    padding: 0.75rem var(--space-row, 0.75rem);
  }
  .task {
    margin: 0;
    font-size: 0.8125rem;
    line-height: 1.5;
    color: var(--foreground);
  }
  .shape {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
