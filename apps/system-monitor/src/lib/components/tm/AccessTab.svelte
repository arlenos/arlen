<script lang="ts">
  /// The Access tab: the sovereign cross-process rollup - who currently holds the
  /// camera, microphone, network, and knowledge access. Each holder jumps to its
  /// process. The same data as the per-process Access detail, grouped by capability.
  import { rollup } from "$lib/stores/detail";
  import type { Process } from "$lib/stores/processes";
  import { Camera, Mic, Globe, Brain, Cog, Cpu } from "lucide-svelte";

  let { list, onJump }: { list: Process[]; onJump: (p: Process) => void } = $props();

  const families = $derived(rollup(list));

  const ICON = { camera: Camera, microphone: Mic, network: Globe, knowledge: Brain } as const;
  const SENSOR = new Set(["camera", "microphone"]);

  function emptyLine(key: string): string {
    return key === "camera"
      ? "Nothing is using your camera."
      : key === "microphone"
        ? "Nothing is using your microphone."
        : key === "network"
          ? "Nothing is using the network."
          : "Nothing is reading the knowledge graph.";
  }
</script>

<div class="at">
  {#each families as f (f.key)}
    {@const FamIcon = ICON[f.key]}
    <section class="fam">
      <h2 class="fam-head" data-lit={SENSOR.has(f.key) && f.holders.length > 0}>
        <FamIcon size={16} strokeWidth={2} />
        <span class="fam-label">{f.label}</span>
        {#if f.holders.length}<span class="fam-count">{f.holders.length}</span>{/if}
      </h2>

      {#if f.holders.length === 0}
        <p class="fam-empty">{emptyLine(f.key)}</p>
      {:else}
        <div class="holders">
          {#each f.holders as h (h.proc.id)}
            <button type="button" class="holder" onclick={() => onJump(h.proc)}>
              {#if h.proc.group === "app"}
                <span class="h-icon avatar" aria-hidden="true">{h.proc.name.charAt(0)}</span>
              {:else if h.proc.group === "background"}
                <span class="h-icon glyph" aria-hidden="true"><Cog size={13} strokeWidth={2} /></span>
              {:else}
                <span class="h-icon glyph" aria-hidden="true"><Cpu size={13} strokeWidth={2} /></span>
              {/if}
              <span class="h-name">{h.proc.name}</span>
              {#if h.detail}<span class="h-detail">{h.detail}</span>{/if}
            </button>
          {/each}
        </div>
      {/if}
    </section>
  {/each}
</div>

<style>
  .at {
    max-width: 42rem;
    padding: 1.5rem 1.75rem;
    display: flex;
    flex-direction: column;
    gap: 1.75rem;
  }
  .fam-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin: 0 0 0.7rem;
    font-size: 0.9375rem;
    font-weight: 600;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .fam-head[data-lit="true"] {
    color: var(--color-warning, #d0a54a);
  }
  .fam-label {
    color: var(--color-fg-primary);
  }
  .fam-head[data-lit="true"] .fam-label {
    color: var(--color-warning, #d0a54a);
  }
  .fam-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1.1rem;
    height: 1.1rem;
    padding: 0 0.3rem;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
    font-size: 0.6875rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 65%, transparent);
  }
  .fam-empty {
    margin: 0;
    font-size: 0.875rem;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .holders {
    display: flex;
    flex-direction: column;
  }
  .holder {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    width: 100%;
    padding: 0.5rem 0.6rem;
    border: none;
    border-radius: var(--radius-input, 8px);
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .holder:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .h-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.2rem;
    height: 1.2rem;
    flex-shrink: 0;
  }
  .h-icon.avatar {
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .h-icon.glyph {
    color: color-mix(in srgb, var(--color-fg-primary) 38%, transparent);
  }
  .h-name {
    font-size: 0.875rem;
    color: var(--color-fg-primary);
    flex-shrink: 0;
  }
  .h-detail {
    flex: 1;
    min-width: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--color-fg-primary) 48%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
  }
</style>
