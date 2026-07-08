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
      <div class="fam-head" data-lit={SENSOR.has(f.key) && f.holders.length > 0}>
        <FamIcon size={12} strokeWidth={2} />
        <span class="fam-label">{f.label}</span>
        {#if f.holders.length}<span class="fam-count">{f.holders.length}</span>{/if}
      </div>

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
  /* Match the process-table register: dense, small uppercase group headers, tight
     tabular rows - not an airy feature section. */
  .at {
    font-size: 0.8125rem;
    padding: 0.4rem 0.4rem 2rem;
  }
  .fam-head {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.7rem 0.6rem 0.3rem;
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .fam-head[data-lit="true"] {
    color: var(--color-warning, #d0a54a);
  }
  .fam-count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 1rem;
    height: 1rem;
    padding: 0 0.25rem;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    font-size: 0.625rem;
    font-weight: 500;
    letter-spacing: 0;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .fam-empty {
    margin: 0;
    padding: 0.35rem 0.6rem;
    color: color-mix(in srgb, var(--color-fg-primary) 42%, transparent);
  }
  .holders {
    display: flex;
    flex-direction: column;
  }
  .holder {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    padding: 0.4rem 0.6rem;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
  }
  .holder:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
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
    color: var(--color-fg-primary);
    flex-shrink: 0;
  }
  .h-detail {
    flex: 1;
    min-width: 0;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
