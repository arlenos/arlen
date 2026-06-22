<script lang="ts">
  /// Dispatches a block's body on `body_kind` (the backend's one bit
  /// per block, terminal-ui-plan.md §3): `grid` reserves the
  /// transparent cell region the compositor paints through;
  /// everything else renders a GUI component inside the output
  /// frame. `body` is opaque to the contract — each branch narrows
  /// it locally and falls back to the grid when the shape
  /// disappoints (never throws on payload, never renders markup
  /// from it).
  import { GridRegion } from "@arlen/ui-kit/components/console";
  import type { Block, GridCell } from "$lib/contract";
  import OutputFrame from "./OutputFrame.svelte";
  import TableLens from "./TableLens.svelte";
  import ImageBlock from "./ImageBlock.svelte";
  import LinkRender from "./LinkRender.svelte";
  import ArtifactCard from "./ArtifactCard.svelte";

  let {
    block,
    tableLens = false,
  }: {
    block: Block;
    /// The table lens is off by default — the block shows its text
    /// grid until the user opts in via the header toggle
    /// (terminal.md §4.8: text stays the truth).
    tableLens?: boolean;
  } = $props();

  /// The block's own captured output, as the per-cell grid the engine
  /// recorded between the command's marks (its "grid inside the
  /// block"). Painted directly so multi-line output is preserved in
  /// full, not truncated to the small live screen. `null` until the
  /// command finishes and the host attaches its cells, or if the
  /// payload shape disappoints (then the placeholder shows).
  const gridCells = $derived.by(() => {
    const b = block.body as { cells?: unknown } | null;
    if (!Array.isArray(b?.cells) || b.cells.length === 0) return null;
    return b.cells as GridCell[][];
  });

  /// A running command has not been captured yet: its output streams in the
  /// live region below the blocks, so its block body shows nothing (no stale
  /// placeholder under a running command). The capture is attached when it
  /// finishes.
  const running = $derived(
    block.exit_code === null && block.duration_ms === null,
  );

  /// The stand-in height for a FINISHED grid block with no attached cells (a
  /// host without the capture): the labelled placeholder keeps proportions real.
  const gridRows = $derived.by(() => {
    const b = block.body as { rows?: number } | null;
    return typeof b?.rows === "number" && b.rows > 0 ? b.rows : 1;
  });

  const tableBody = $derived.by(() => {
    if (block.body_kind !== "table") return null;
    const b = block.body as { columns?: unknown; cells?: unknown } | null;
    if (!Array.isArray(b?.columns) || !Array.isArray(b?.cells)) return null;
    return {
      columns: b.columns.map(String),
      cells: (b.cells as unknown[]).map((r) =>
        Array.isArray(r) ? r.map(String) : [String(r)],
      ),
    };
  });

  const imageBody = $derived.by(() => {
    if (block.body_kind !== "image") return null;
    const b = block.body as
      | { src?: unknown; alt?: unknown; width?: unknown; height?: unknown }
      | null;
    if (typeof b?.src !== "string") return null;
    return {
      src: b.src,
      alt: typeof b.alt === "string" ? b.alt : "",
      width: typeof b.width === "number" ? b.width : null,
      height: typeof b.height === "number" ? b.height : null,
    };
  });

  const linkBody = $derived.by(() => {
    if (block.body_kind !== "link") return null;
    const b = block.body as { url?: unknown; text?: unknown } | null;
    if (typeof b?.url !== "string") return null;
    return { url: b.url, text: typeof b.text === "string" ? b.text : b.url };
  });

  const artifactBody = $derived.by(() => {
    if (block.body_kind !== "artifact") return null;
    const b = block.body as
      | { title?: unknown; kind?: unknown; summary?: unknown }
      | null;
    if (typeof b?.title !== "string") return null;
    return {
      title: b.title,
      kind: typeof b.kind === "string" ? b.kind : "artifact",
      summary: typeof b.summary === "string" ? b.summary : null,
    };
  });
</script>

{#if block.body_kind === "grid" || (block.body_kind === "table" && !tableLens)}
  {#if gridCells}
    <GridRegion cells={gridCells} />
  {:else if !running}
    <GridRegion
      rows={gridRows}
      placeholder={`terminal output, ${gridRows} ${gridRows === 1 ? "line" : "lines"}`}
    />
  {/if}
{:else if block.body_kind === "table" && tableBody}
  <OutputFrame>
    <TableLens columns={tableBody.columns} cells={tableBody.cells} />
  </OutputFrame>
{:else if block.body_kind === "image" && imageBody}
  <OutputFrame>
    <ImageBlock
      src={imageBody.src}
      alt={imageBody.alt}
      width={imageBody.width}
      height={imageBody.height}
    />
  </OutputFrame>
{:else if block.body_kind === "link" && linkBody}
  <OutputFrame>
    <LinkRender url={linkBody.url} text={linkBody.text} />
  </OutputFrame>
{:else if block.body_kind === "artifact" && artifactBody}
  <OutputFrame>
    <ArtifactCard
      title={artifactBody.title}
      kind={artifactBody.kind}
      summary={artifactBody.summary}
    />
  </OutputFrame>
{:else if block.body_kind === "widget"}
  <div class="bb-inert">Interactive widgets are not available yet.</div>
{:else}
  <!-- A GUI kind whose payload did not match its shape: fall back
       to the reserved grid so the raw text remains reachable. -->
  <GridRegion rows={gridRows} placeholder="terminal output" />
{/if}

<style>
  .bb-inert {
    padding: 6px 8px;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
</style>
