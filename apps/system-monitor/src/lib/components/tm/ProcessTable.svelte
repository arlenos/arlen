<script lang="ts">
  /// The process table: the task-manager landing. A dense, sortable, heat-coloured
  /// list grouped into Apps / Background / System, a Stop on every row. The Arlen
  /// daemons + the AI agent sit in Background as ordinary rows. No verdict page.
  import { ChevronRight, Square, Cog, Cpu } from "lucide-svelte";
  import type { Process, ProcGroup, ProcStatus, SortKey } from "$lib/stores/processes";

  let {
    list,
    filter = "",
    flatten = false,
    selectedId,
    onStop,
    onSelect,
  }: {
    list: Process[];
    filter?: string;
    flatten?: boolean;
    selectedId?: number;
    onStop: (id: number) => void;
    onSelect?: (p: Process) => void;
  } = $props();

  let sortKey = $state<SortKey>("cpu");
  let sortDir = $state<"asc" | "desc">("desc");
  let expanded = $state<Set<number>>(new Set());

  function sortBy(key: SortKey) {
    if (sortKey === key) sortDir = sortDir === "desc" ? "asc" : "desc";
    else {
      sortKey = key;
      sortDir = key === "name" || key === "status" ? "asc" : "desc";
    }
  }
  function toggle(id: number) {
    const next = new Set(expanded);
    next.has(id) ? next.delete(id) : next.add(id);
    expanded = next;
  }

  const GROUPS: { key: ProcGroup; label: string }[] = [
    { key: "app", label: "Apps" },
    { key: "background", label: "Background" },
    { key: "system", label: "System" },
  ];
  const STATUS_LABEL: Record<ProcStatus, string> = {
    running: "Running",
    "not-responding": "Not responding",
    suspended: "Suspended",
  };

  function matches(p: Process): boolean {
    if (!filter.trim()) return true;
    const q = filter.toLowerCase();
    if (p.name.toLowerCase().includes(q)) return true;
    return (p.children ?? []).some((c) => c.name.toLowerCase().includes(q));
  }
  function cmp(a: Process, b: Process): number {
    const dir = sortDir === "desc" ? -1 : 1;
    if (sortKey === "name") return dir * a.name.localeCompare(b.name);
    if (sortKey === "status") return dir * a.status.localeCompare(b.status);
    return dir * (a[sortKey] - b[sortKey]);
  }

  type DisplayItem =
    | { kind: "group"; label: string }
    | { kind: "proc"; proc: Process; depth: number; expandable: boolean; open: boolean };

  const items = $derived.by<DisplayItem[]>(() => {
    const out: DisplayItem[] = [];
    for (const g of GROUPS) {
      const rows = list.filter((p) => p.group === g.key && matches(p)).sort(cmp);
      if (rows.length === 0) continue;
      out.push({ kind: "group", label: g.label });
      for (const p of rows) {
        const kids = p.children ?? [];
        if (flatten && kids.length) {
          for (const c of [...kids].sort(cmp)) out.push({ kind: "proc", proc: c, depth: 0, expandable: false, open: false });
        } else {
          const open = expanded.has(p.id);
          out.push({ kind: "proc", proc: p, depth: 0, expandable: kids.length > 0, open });
          if (open) for (const c of [...kids].sort(cmp)) out.push({ kind: "proc", proc: c, depth: 1, expandable: false, open: false });
        }
      }
    }
    return out;
  });

  // Column totals for the header (the Windows aggregate-in-header). Sum the top-level
  // rows (app aggregates + background + system), not the expanded children.
  const totals = $derived.by(() => {
    let cpu = 0, memMB = 0, diskKBs = 0, netKBs = 0;
    for (const p of list) {
      cpu += p.cpu;
      memMB += p.memMB;
      diskKBs += p.diskKBs;
      netKBs += p.netKBs;
    }
    return { cpu, memMB, diskKBs, netKBs };
  });

  function mem(mb: number): string {
    return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;
  }
  function rate(kbs: number): string {
    if (kbs === 0) return "";
    return kbs >= 1024 ? `${(kbs / 1024).toFixed(1)} MB/s` : `${Math.round(kbs)} KB/s`;
  }
  // Heat intensity 0..1 for a cell, by a per-column scale.
  function heat(v: number, scale: number): number {
    return Math.max(0, Math.min(1, v / scale));
  }
  function ariaSort(key: SortKey): "ascending" | "descending" | "none" {
    return sortKey === key ? (sortDir === "asc" ? "ascending" : "descending") : "none";
  }
</script>

<div class="pt" role="table" aria-label="Processes">
  <div class="head" role="row">
    <button class="h name" class:sorted={sortKey === "name"} role="columnheader" aria-sort={ariaSort("name")} onclick={() => sortBy("name")}>
      Name
      {#if sortKey === "name"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}
    </button>
    <button class="h" class:sorted={sortKey === "status"} role="columnheader" aria-sort={ariaSort("status")} onclick={() => sortBy("status")}>
      Status
    </button>
    <button class="h num" class:sorted={sortKey === "cpu"} role="columnheader" aria-sort={ariaSort("cpu")} onclick={() => sortBy("cpu")}>
      <span class="h-label">CPU {#if sortKey === "cpu"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}</span>
      <span class="h-total">{totals.cpu.toFixed(0)}%</span>
    </button>
    <button class="h num" class:sorted={sortKey === "memMB"} role="columnheader" aria-sort={ariaSort("memMB")} onclick={() => sortBy("memMB")}>
      <span class="h-label">Memory {#if sortKey === "memMB"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}</span>
      <span class="h-total">{mem(totals.memMB)}</span>
    </button>
    <button class="h num" class:sorted={sortKey === "diskKBs"} role="columnheader" aria-sort={ariaSort("diskKBs")} onclick={() => sortBy("diskKBs")}>
      <span class="h-label">Disk {#if sortKey === "diskKBs"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}</span>
      <span class="h-total">{rate(totals.diskKBs) || "0"}</span>
    </button>
    <button class="h num" class:sorted={sortKey === "netKBs"} role="columnheader" aria-sort={ariaSort("netKBs")} onclick={() => sortBy("netKBs")}>
      <span class="h-label">Network {#if sortKey === "netKBs"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}</span>
      <span class="h-total">{rate(totals.netKBs) || "0"}</span>
    </button>
    <span class="h stop" aria-hidden="true"></span>
  </div>

  <div class="body">
    {#each items as it, i (it.kind === "group" ? `g-${it.label}` : `p-${it.proc.id}-${it.depth}`)}
      {#if it.kind === "group"}
        <div class="grouprow" role="row"><span>{it.label}</span></div>
      {:else}
        {@const p = it.proc}
        <div
          class="row"
          class:child={it.depth > 0}
          class:selected={p.id === selectedId}
          role="row"
          tabindex="0"
          onclick={() => onSelect?.(p)}
          onkeydown={(e) => {
            if (e.key === "Enter") onSelect?.(p);
          }}
        >
          <div class="cell name" role="cell">
            {#if it.expandable}
              <button
                class="twist"
                class:open={it.open}
                aria-label="Expand"
                onclick={(e) => {
                  e.stopPropagation();
                  toggle(p.id);
                }}
              >
                <ChevronRight size={13} strokeWidth={2} />
              </button>
            {:else}
              <span class="twist-spacer"></span>
            {/if}
            {#if it.depth > 0}
              <span class="picon dot" aria-hidden="true"></span>
            {:else if p.group === "app"}
              <span class="picon avatar" aria-hidden="true">{p.name.charAt(0)}</span>
            {:else if p.group === "background"}
              <span class="picon glyph" aria-hidden="true"><Cog size={13} strokeWidth={2} /></span>
            {:else}
              <span class="picon glyph" aria-hidden="true"><Cpu size={13} strokeWidth={2} /></span>
            {/if}
            <span class="pname">{p.name}</span>
          </div>
          <div class="cell status" role="cell" data-status={p.status}>{STATUS_LABEL[p.status]}</div>
          <div class="cell num" role="cell" style="--heat: {heat(p.cpu, 25)}">{p.cpu.toFixed(1)}%</div>
          <div class="cell num" role="cell" style="--heat: {heat(p.memMB, 2200)}">{mem(p.memMB)}</div>
          <div class="cell num muted" role="cell">{rate(p.diskKBs)}</div>
          <div class="cell num muted" role="cell">{rate(p.netKBs)}</div>
          <div class="cell stop" role="cell">
            <button
              class="stop-btn"
              aria-label={`Stop ${p.name}`}
              title="Stop"
              onclick={(e) => {
                e.stopPropagation();
                onStop(p.id);
              }}
            >
              <Square size={12} strokeWidth={2.5} />
            </button>
          </div>
        </div>
      {/if}
    {/each}
  </div>
</div>

<style>
  .pt {
    font-size: 0.8125rem;
  }
  .head,
  .row,
  .grouprow {
    display: grid;
    grid-template-columns: minmax(12rem, 1fr) 8.5rem 5rem 6rem 6rem 6.5rem 2rem;
    align-items: center;
  }
  .head {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--color-bg-app, #0f0f0f);
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
  }
  .h {
    display: flex;
    align-items: baseline;
    gap: 0.3rem;
    padding: 0.5rem 0.6rem;
    border: none;
    background: transparent;
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    cursor: pointer;
    text-align: left;
  }
  .h:hover {
    color: color-mix(in srgb, var(--color-fg-primary) 75%, transparent);
  }
  .h.sorted {
    color: var(--color-fg-primary);
  }
  .h.num {
    flex-direction: column;
    align-items: flex-end;
    gap: 0.05rem;
  }
  .h-label {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
  }
  .h-total {
    font-size: 0.625rem;
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .arrow {
    font-size: 0.5rem;
  }

  .grouprow {
    padding: 0.55rem 0.6rem 0.25rem;
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
    display: block;
  }

  .row {
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
  }
  .row {
    cursor: pointer;
    outline: none;
  }
  .row:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 4%, transparent);
  }
  .row.selected {
    background: color-mix(in srgb, var(--color-fg-primary) 9%, transparent);
  }
  .cell {
    padding: 0.4rem 0.6rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cell.name {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    color: var(--color-fg-primary);
  }
  .row.child .pname {
    color: color-mix(in srgb, var(--color-fg-primary) 62%, transparent);
  }
  .twist {
    display: inline-flex;
    padding: 0;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
    cursor: pointer;
  }
  .twist :global(svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .twist.open :global(svg) {
    transform: rotate(90deg);
  }
  .twist-spacer {
    width: 13px;
    flex-shrink: 0;
  }
  .picon {
    flex-shrink: 0;
    width: 1.2rem;
    height: 1.2rem;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .picon.avatar {
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .picon.glyph {
    color: color-mix(in srgb, var(--color-fg-primary) 38%, transparent);
  }
  .picon.dot::before {
    content: "";
    width: 0.3rem;
    height: 0.3rem;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-fg-primary) 28%, transparent);
  }
  .pname {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .cell.status {
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .cell.status[data-status="not-responding"] {
    color: var(--color-warning, #d0a54a);
  }
  .cell.status[data-status="suspended"] {
    color: color-mix(in srgb, var(--color-fg-primary) 40%, transparent);
  }
  .cell.num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--color-fg-primary);
    /* Monochrome heat: the busier the cell, the brighter its wash. */
    background: color-mix(in srgb, var(--color-fg-primary) calc(var(--heat, 0) * 16%), transparent);
  }
  .cell.num.muted {
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
    background: transparent;
  }
  .cell.stop {
    padding: 0;
    display: flex;
    justify-content: center;
  }
  .stop-btn {
    display: inline-flex;
    padding: 0.25rem;
    border: none;
    background: transparent;
    border-radius: var(--radius-chip, 4px);
    color: color-mix(in srgb, var(--color-fg-primary) 30%, transparent);
    cursor: pointer;
  }
  .row:hover .stop-btn {
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
  }
  .stop-btn:hover {
    background: color-mix(in srgb, var(--color-error, #c96a6a) 16%, transparent);
    color: var(--color-error, #c96a6a);
  }
</style>
