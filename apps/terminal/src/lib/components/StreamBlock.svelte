<script lang="ts">
  /// One block in the stream: wires the contract block onto the kit
  /// ConsoleBlock and owns the per-block view state — today that is
  /// the table lens (off by default; a quiet word toggle in the
  /// header offers the rendered view only when the backend
  /// recognized a table).
  import { ConsoleBlock, GridRegion } from "@arlen/ui-kit/components/console";
  import type { Block } from "$lib/contract";
  import PromptLine from "./PromptLine.svelte";
  import OriginMarker from "./OriginMarker.svelte";
  import BlockBody from "./BlockBody.svelte";

  let { block }: { block: Block } = $props();

  let tableLens = $state(false);

  // The block header shows the shell's own captured prompt line (prompt +
  // echoed command, with its real colours) when the engine captured it - the
  // record is then exactly what was on screen, for any prompt. Older blocks /
  // the mock fall back to the regenerated path+git line.
  const hasPromptCells = $derived(
    Array.isArray(block.prompt_cells) && block.prompt_cells.length > 0,
  );
</script>

<ConsoleBlock
  command={hasPromptCells ? "" : block.command}
  promptFull={hasPromptCells}
  exitCode={block.exit_code}
  durationMs={block.duration_ms}
  running={block.exit_code === null && block.duration_ms === null}
>
  {#snippet context()}
    {#if hasPromptCells}
      <GridRegion cells={block.prompt_cells ?? []} />
    {:else}
      <PromptLine cwd={block.cwd} git={block.git} />
    {/if}
  {/snippet}
  {#snippet marker()}
    <OriginMarker origin={block.origin} />
  {/snippet}
  {#snippet lens()}
    {#if block.body_kind === "table"}
      <button
        class="lens-btn"
        aria-label={tableLens ? "Show the plain text" : "Show as table"}
        aria-pressed={tableLens}
        onclick={() => (tableLens = !tableLens)}
      >
        table
      </button>
    {/if}
  {/snippet}
  <BlockBody {block} {tableLens} />
</ConsoleBlock>

<style>
  /* The same quiet word-chip language as the sidebar filter chips. */
  .lens-btn {
    display: inline-flex;
    align-items: center;
    height: var(--height-control-compact, 24px);
    padding: 0 7px;
    border: 1px solid var(--control-border);
    border-radius: var(--radius-chip);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    font-size: 0.75rem;
    font-weight: 500;
    transition:
      background-color var(--duration-micro, 100ms) var(--ease-out, ease),
      color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .lens-btn:hover {
    color: var(--foreground);
  }
  .lens-btn[aria-pressed="true"] {
    background: color-mix(in srgb, var(--color-accent, var(--primary)) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-accent, var(--primary)) 35%, transparent);
    color: var(--color-accent, var(--primary));
  }
</style>
