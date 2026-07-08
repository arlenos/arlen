<script lang="ts">
  /// The process table: the task-manager landing. A dense, sortable, heat-coloured
  /// list grouped into Apps / Background / System, a Stop on every row. The Arlen
  /// daemons + the AI agent sit in Background as ordinary rows. No verdict page.
  import { ChevronRight, Cog, Cpu, Camera, Mic, Brain } from "lucide-svelte";
  import type { Process, ProcGroup, ProcStatus, SortKey } from "$lib/stores/processes";
  import { sensorsFor } from "$lib/stores/detail";

  let {
    list,
    filter = "",
    flatten = false,
    selectedId,
    onSelect,
    onContextMenu,
  }: {
    list: Process[];
    filter?: string;
    flatten?: boolean;
    selectedId?: number;
    onSelect?: (p: Process) => void;
    onContextMenu?: (p: Process, x: number, y: number) => void;
  } = $props();

  let sortKey = $state<SortKey>("cpu");
  let sortDir = $state<"asc" | "desc">("desc");
  let expanded = $state<Set<number>>(new Set());

  // Keyboard drive (the btop users): one roving tabstop, arrow-key navigation.
  let rootEl = $state<HTMLElement | null>(null);
  let activeId = $state<number | null>(null);

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

  // The focusable rows, in display order - group rows are skipped by the keyboard.
  const procIds = $derived(
    items.filter((it): it is Extract<DisplayItem, { kind: "proc" }> => it.kind === "proc").map((it) => it.proc.id),
  );
  // The single tabstop: the active row, or the first row if none is set yet.
  const activeRowId = $derived(activeId != null && procIds.includes(activeId) ? activeId : procIds[0]);

  function focusRow(id: number) {
    activeId = id;
    requestAnimationFrame(() => {
      (rootEl?.querySelector(`[data-pid="${id}"]`) as HTMLElement | null)?.focus();
    });
  }
  function openMenuAt(el: HTMLElement, p: Process) {
    const r = el.getBoundingClientRect();
    onContextMenu?.(p, r.left + 8, r.bottom);
  }
  function rowKeydown(e: KeyboardEvent, p: Process, expandable: boolean, open: boolean) {
    const ids = procIds;
    const idx = ids.indexOf(p.id);
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        if (idx < ids.length - 1) focusRow(ids[idx + 1]);
        break;
      case "ArrowUp":
        e.preventDefault();
        if (idx > 0) focusRow(ids[idx - 1]);
        break;
      case "Home":
        e.preventDefault();
        if (ids.length) focusRow(ids[0]);
        break;
      case "End":
        e.preventDefault();
        if (ids.length) focusRow(ids[ids.length - 1]);
        break;
      case "ArrowRight":
        if (expandable && !open) {
          e.preventDefault();
          toggle(p.id);
        }
        break;
      case "ArrowLeft":
        if (expandable && open) {
          e.preventDefault();
          toggle(p.id);
        }
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        onSelect?.(p);
        break;
      case "ContextMenu":
        e.preventDefault();
        openMenuAt(e.currentTarget as HTMLElement, p);
        break;
      case "F10":
        if (e.shiftKey) {
          e.preventDefault();
          openMenuAt(e.currentTarget as HTMLElement, p);
        }
        break;
    }
  }

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
  // A limited process's CPU is capped; a paused one reads 0 (frozen). No heat on
  // either - it's throttled, not hot.
  const LIMIT_CAP = 10;
  function dispCpu(p: Process): number {
    return p.paused ? 0 : p.limited ? Math.min(p.cpu, LIMIT_CAP) : p.cpu;
  }
  function dispHeat(p: Process): number {
    return p.paused || p.limited ? 0 : heat(p.cpu, 25);
  }
  function ariaSort(key: SortKey): "ascending" | "descending" | "none" {
    return sortKey === key ? (sortDir === "asc" ? "ascending" : "descending") : "none";
  }
</script>

<div class="pt" role="grid" aria-label="Processes" bind:this={rootEl}>
  <div class="head" role="row">
    <button class="h name" class:sorted={sortKey === "name"} role="columnheader" aria-sort={ariaSort("name")} onclick={() => sortBy("name")}>
      Name
      {#if sortKey === "name"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}
    </button>
    <button class="h" class:sorted={sortKey === "status"} role="columnheader" aria-sort={ariaSort("status")} onclick={() => sortBy("status")}>
      Status
    </button>
    <span class="h access" role="columnheader">Access</span>
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
  </div>

  <div class="body">
    {#each items as it, i (it.kind === "group" ? `g-${it.label}` : `p-${it.proc.id}-${it.depth}`)}
      {#if it.kind === "group"}
        <div class="grouprow" role="presentation"><span>{it.label}</span></div>
      {:else}
        {@const p = it.proc}
        {@const sensors = sensorsFor(p.name)}
        <div
          class="row"
          class:child={it.depth > 0}
          class:selected={p.id === selectedId}
          role="row"
          data-pid={p.id}
          tabindex={p.id === activeRowId ? 0 : -1}
          onclick={() => {
            activeId = p.id;
            onSelect?.(p);
          }}
          oncontextmenu={(e) => {
            e.preventDefault();
            activeId = p.id;
            onContextMenu?.(p, e.clientX, e.clientY);
          }}
          onkeydown={(e) => rowKeydown(e, p, it.expandable, it.open)}
        >
          <div class="cell name" role="gridcell">
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
          <div class="cell status" role="gridcell" data-status={p.paused ? "suspended" : p.status}>
            <span>{p.paused ? "Suspended" : STATUS_LABEL[p.status]}</span>
            {#if p.limited && !p.paused}<span class="limtag">Limited</span>{/if}
          </div>
          <div class="cell access" role="gridcell">
            {#if sensors.camera}<Camera size={13} strokeWidth={2} />{/if}
            {#if sensors.mic}<Mic size={13} strokeWidth={2} />{/if}
            {#if sensors.knowledge}<span class="kg-glyph"><Brain size={13} strokeWidth={2} /></span>{/if}
          </div>
          <div class="cell num" role="gridcell" style="--heat: {dispHeat(p)}">{dispCpu(p).toFixed(1)}%</div>
          <div class="cell num" role="gridcell" style="--heat: {heat(p.memMB, 2200)}">{mem(p.memMB)}</div>
          <div class="cell num muted" role="gridcell">{rate(p.diskKBs)}</div>
          <div class="cell num muted" role="gridcell">{rate(p.netKBs)}</div>
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
    grid-template-columns: minmax(12rem, 1fr) 8.5rem 4rem 5rem 6rem 6rem 6.5rem;
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
  /* Keyboard focus must be obvious - an inset ring in the fg tone (not browser blue). */
  .row:focus-visible {
    box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
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
    display: flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .limtag {
    padding: 0.02rem 0.3rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    font-size: 0.625rem;
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
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
  .h.access {
    justify-content: center;
  }
  .cell.access {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 0.25rem;
    color: var(--color-warning, #d0a54a);
  }
  /* Knowledge access is visible but not a physical-surveillance alarm - a neutral
     tone, distinct from the amber camera/mic. */
  .kg-glyph {
    display: inline-flex;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
</style>
