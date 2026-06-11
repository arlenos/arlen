<script lang="ts">
  /// One block in the stream: wires the contract block onto the kit
  /// ConsoleBlock and owns the per-block view state — today that is
  /// the table lens (off by default; the quiet grid toggle in the
  /// header offers the rendered view only when the backend
  /// recognized a table).
  import { Grid3x3 } from "lucide-svelte";
  import { ConsoleBlock } from "@arlen/ui-kit/components/console";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import type { Block } from "$lib/contract";
  import PromptLine from "./PromptLine.svelte";
  import OriginMarker from "./OriginMarker.svelte";
  import BlockBody from "./BlockBody.svelte";

  let { block }: { block: Block } = $props();

  let tableLens = $state(false);
</script>

<ConsoleBlock
  command={block.command}
  exitCode={block.exit_code}
  durationMs={block.duration_ms}
  running={block.exit_code === null && block.duration_ms === null}
  originLabel={block.origin === "agent" ? "agent" : null}
>
  {#snippet context()}
    <PromptLine cwd={block.cwd} git={block.git} />
  {/snippet}
  {#snippet marker()}
    <OriginMarker origin={block.origin} />
  {/snippet}
  {#snippet lens()}
    {#if block.body_kind === "table"}
      <Tooltip.Root>
        <Tooltip.Trigger>
          {#snippet child({ props })}
            <button
              {...props}
              class="lens-btn"
              class:on={tableLens}
              aria-label="Show as table"
              aria-pressed={tableLens}
              onclick={() => (tableLens = !tableLens)}
            >
              <Grid3x3 size={13} strokeWidth={1.75} />
            </button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.TooltipContent side="bottom">
          {tableLens ? "Show the plain text" : "Show as table"}
        </Tooltip.TooltipContent>
      </Tooltip.Root>
    {/if}
  {/snippet}
  <BlockBody {block} {tableLens} />
</ConsoleBlock>

<style>
  .lens-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    transition:
      background-color var(--duration-micro, 100ms) var(--ease-out, ease),
      color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .lens-btn:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .lens-btn.on {
    background: color-mix(in srgb, var(--color-accent, var(--primary)) 15%, transparent);
    color: var(--color-accent, var(--primary));
  }
</style>
