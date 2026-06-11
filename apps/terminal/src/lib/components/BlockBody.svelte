<script lang="ts">
  /// Dispatches a block's body on `body_kind` (the backend's one bit
  /// per block, terminal-ui-plan.md §3): `grid` reserves the
  /// transparent cell region the compositor paints through;
  /// everything else renders a GUI component. `body` is opaque to
  /// the contract — each branch narrows it locally and renders
  /// nothing when the shape disappoints (never throws on payload).
  import { GridRegion } from "@arlen/ui-kit/components/console";
  import type { Block } from "$lib/contract";

  let { block }: { block: Block } = $props();

  /// Until the compositor subsurface lands, every host runs without
  /// the grid paint — the labelled stand-in keeps proportions real.
  /// The subsurface wiring removes the placeholder, nothing else.
  const gridRows = $derived.by(() => {
    const b = block.body as { rows?: number } | null;
    return typeof b?.rows === "number" && b.rows > 0 ? b.rows : 1;
  });
</script>

{#if block.body_kind === "grid"}
  <GridRegion
    rows={gridRows}
    placeholder={`terminal output, ${gridRows} ${gridRows === 1 ? "line" : "lines"}`}
  />
{:else}
  <!-- GUI block kinds land in the next increment; until then the
       body states what it will be instead of pretending. -->
  <div class="bb-pending">{block.body_kind} block</div>
{/if}

<style>
  .bb-pending {
    padding: 6px 8px;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
  }
</style>
