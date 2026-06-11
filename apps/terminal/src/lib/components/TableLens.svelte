<script lang="ts">
  /// The rendered table view of a table-recognized block. Reached
  /// only through the quiet lens toggle in the block header — text
  /// stays the truth, this is a render-only view over it
  /// (terminal.md §4.8). Cells are plain text by invariant.
  let {
    columns,
    cells,
  }: {
    columns: string[];
    cells: string[][];
  } = $props();

  /// Columns whose cells all read as numbers align right, so values
  /// line up for comparison.
  const numeric = $derived(
    columns.map(
      (_, i) =>
        cells.length > 0 &&
        cells.every((row) => /^-?[\d.,]+%?$/.test((row[i] ?? "").trim())),
    ),
  );
</script>

<table class="table-lens">
  <thead>
    <tr>
      {#each columns as col, i}
        <th class:num={numeric[i]}>{col}</th>
      {/each}
    </tr>
  </thead>
  <tbody>
    {#each cells as row}
      <tr>
        {#each row as cell, i}
          <td class:num={numeric[i]}>{cell}</td>
        {/each}
      </tr>
    {/each}
  </tbody>
</table>

<style>
  .table-lens {
    width: 100%;
    border-collapse: collapse;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.8125rem;
    line-height: 1.5;
  }

  th {
    text-align: left;
    font-weight: 600;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    padding: 2px 14px 4px 0;
    border-bottom: 1px solid
      color-mix(in srgb, var(--foreground) 12%, transparent);
  }

  td {
    padding: 3px 14px 3px 0;
    color: var(--foreground);
    border-bottom: 1px solid
      color-mix(in srgb, var(--foreground) 5%, transparent);
  }

  tbody tr:last-child td {
    border-bottom: none;
  }

  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
</style>
