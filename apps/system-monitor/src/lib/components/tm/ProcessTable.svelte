<script lang="ts">
  /// The process table: the task-manager landing. A dense, sortable, heat-coloured
  /// list grouped into Apps / Background / System. The Arlen daemons + the AI agent
  /// sit in Background as ordinary rows. No verdict page.
  ///
  /// STOP LIVES IN THE ROW MENU, not on the row. This header said "a Stop on every
  /// row" until 16 August, which stopped being true on 8 July when the per-row
  /// button was moved into `RowMenu` to make space for the access column. Right
  /// click, or the ContextMenu key / Shift+F10 on the focused row, which is the
  /// platform convention and keeps the action keyboard-reachable.
  ///
  /// NB `system-monitor-plan.md` (a) lists "Stop visible on every row (not buried)"
  /// among the first-class interactions, so the placement is a live disagreement
  /// between the plan and the app rather than a settled question - raised in
  /// `coder-reports.md` on 16 August.
  import { ChevronRight, Cog, Cpu, Camera, Mic, Brain } from "lucide-svelte";
  import type { Process, ProcGroup, ProcStatus, SortKey } from "$lib/stores/processes";
  import { sensorsFor } from "$lib/stores/detail";
  import { pinnedOrder, rowMatches } from "$lib/freeze";
  import { t, locale } from "$lib/i18n/messages";
  import { formatDecimal } from "@arlen/ui-kit/i18n";

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

  const GROUPS: { key: ProcGroup; id: string }[] = [
    { key: "app", id: "tm.group.app" },
    { key: "background", id: "tm.group.background" },
    { key: "system", id: "tm.group.system" },
  ];
  const STATUS_ID: Record<ProcStatus, string> = {
    running: "tm.status.running",
    "not-responding": "tm.status.notResponding",
    suspended: "tm.status.suspended",
  };

  // The rule lives in `$lib/freeze` so the child-name clause can be tested: on
  // this machine a browser's children carry the browser's own name, so a drive
  // cannot tell "matched itself" from "matched a child".
  const matches = (p: Process) => rowMatches(p, filter);
  function cmp(a: Process, b: Process): number {
    const dir = sortDir === "desc" ? -1 : 1;
    if (sortKey === "name") return dir * a.name.localeCompare(b.name);
    if (sortKey === "status") return dir * a.status.localeCompare(b.status);
    return dir * (a[sortKey] - b[sortKey]);
  }

  type DisplayItem =
    | { kind: "group"; id: string }
    | { kind: "proc"; proc: Process; depth: number; expandable: boolean; open: boolean };

  // FREEZE-THE-REFRESH (plan (a)). Holding the modifier pins the row ORDER, not
  // the data: the poll keeps running and the figures keep arriving, only the
  // positions stop moving. Stopping the poll instead would show a two-second-old
  // snapshot as if it were now.
  //
  // The pin is captured per group at the moment the key goes down, and it is
  // captured from the sorted ids rather than from the raw list, because the sort
  // is what moves rows about.
  let frozen = $state(false);
  let pinned = $state<Map<string, number[]>>(new Map());

  function onFreezeKey(e: KeyboardEvent) {
    // Shift, not Ctrl or Alt: Ctrl is the platform's own accelerator prefix and
    // Alt reaches the window menu on some compositors, while Shift alone does
    // nothing here otherwise.
    if (e.key !== "Shift" || e.repeat) return;
    if (e.type === "keydown") {
      frozen = true;
    } else {
      frozen = false;
      pinned = new Map();
    }
  }

  const items = $derived.by<DisplayItem[]>(() => {
    const out: DisplayItem[] = [];
    for (const g of GROUPS) {
      let rows = list.filter((p) => p.group === g.key && matches(p)).sort(cmp);
      if (frozen) {
        const held = pinned.get(g.key);
        if (held) {
          rows = pinnedOrder(rows, held);
        } else {
          // First derive after the key went down: this order becomes the pin.
          pinned.set(g.key, rows.map((r) => r.id));
        }
      }
      if (rows.length === 0) continue;
      out.push({ kind: "group", id: g.id });
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

  // A failed read leaves nothing to sum, and a sum of nothing is zero - which in
  // this header reads as "0% CPU, 0 MB", a measurement of the machine, sitting
  // directly under a line saying the processes could not be read. Seen on 9
  // August in the first desktop-width sweep. Blank says nothing, which is what we
  // know, and it is already this file's convention: `rate()` renders a zero rate
  // as blank rather than as a nought.
  const totalCpu = $derived(list.length ? `${formatDecimal(totals.cpu, 0, $locale)}%` : "");
  const totalMem = $derived(list.length ? mem(totals.memMB) : "");
  const totalDisk = $derived(list.length ? rate(totals.diskKBs) || "0" : "");
  /// Whether per-process network is measured at all on this system.
  ///
  /// It is not, today: `/proc` carries no per-process byte counters, the backend
  /// reports 0 and documents why, and eBPF/cgroup attribution is the piece that
  /// would change it. Until then the column shows a dash rather than a zero,
  /// because a zero is a claim about the process and a dash is a statement about
  /// the column. Flip this to a real signal from the backend the day the
  /// attribution lands, rather than deleting the column now - the plan lists
  /// Network among the default columns.
  const netMeasured = false;

  /// Whether the Access column reflects the machine. It does not.
  ///
  /// `sensorsFor` reads a hand-keyed table in `stores/detail.ts` matched on process
  /// NAME - the file says so itself, calling it a stand-in for the permission
  /// profile - so on a real machine it lights up for nothing, and would light a
  /// camera icon for any process that happened to be called "Meet". A security
  /// column that answers from a name table is worse than one that stays quiet:
  /// blank reads as "nothing is using your camera", which nobody checked.
  ///
  /// So the icons are held back until the column is driven by the profile data
  /// Settings already derives (`stores/grants.ts`), and the header says it is not
  /// measured. The rendering stays so that wiring the real source is a one-line
  /// flip rather than a rebuild.
  const accessMeasured = false;
  const totalNet = $derived(netMeasured && list.length ? rate(totals.netKBs) || "0" : "");

  // `$locale` rather than the default, so the table re-renders on a language
  // switch: a template calling a function that reads the store internally has no
  // dependency on it and would keep the first render's convention.
  function mem(mb: number): string {
    return mb >= 1024
      ? `${formatDecimal(mb / 1024, 1, $locale)} GB`
      : `${formatDecimal(Math.round(mb), 0, $locale)} MB`;
  }
  function rate(kbs: number): string {
    if (kbs === 0) return "";
    return kbs >= 1024
      ? `${formatDecimal(kbs / 1024, 1, $locale)} MB/s`
      : `${formatDecimal(Math.round(kbs), 0, $locale)} KB/s`;
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

<svelte:window onkeydown={onFreezeKey} onkeyup={onFreezeKey} />

<div class="pt" role="grid" data-frozen={frozen ? "yes" : "no"} aria-label={$t("tm.grid.label", { count: procIds.length })} bind:this={rootEl}>
  <div class="head" role="row">
    <span class="hcell" role="columnheader" aria-sort={ariaSort("name")}><button class="h name" class:sorted={sortKey === "name"} aria-label={$t("tm.col.name")} onclick={() => sortBy("name")}>
      {$t("tm.col.name")}
      {#if sortKey === "name"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}
    </button></span>
    <span class="hcell" role="columnheader" aria-sort={ariaSort("status")}><button class="h" class:sorted={sortKey === "status"} aria-label={$t("tm.col.status")} onclick={() => sortBy("status")}>
      {$t("tm.col.status")}
    </button></span>
    <span class="h access" role="columnheader" aria-label={$t("tm.col.access")} title={accessMeasured ? undefined : $t("tm.col.accessUnavailable")}>
      {$t("tm.col.access")}{#if !accessMeasured}<span class="h-total unmeasured">{$t("tm.col.notMeasured")}</span>{/if}
    </span>
    <span class="hcell" role="columnheader" aria-sort={ariaSort("cpu")}><button class="h num" class:sorted={sortKey === "cpu"} aria-label={totalCpu ? $t("tm.col.withTotal", { col: $t("tm.col.cpu"), total: totalCpu }) : $t("tm.col.cpu")} onclick={() => sortBy("cpu")}>
      <span class="h-label">{$t("tm.col.cpu")} {#if sortKey === "cpu"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}</span>
      <span class="h-total">{totalCpu}</span>
    </button></span>
    <span class="hcell" role="columnheader" aria-sort={ariaSort("memMB")}><button class="h num" class:sorted={sortKey === "memMB"} aria-label={totalMem ? $t("tm.col.withTotal", { col: $t("tm.col.memory"), total: totalMem }) : $t("tm.col.memory")} onclick={() => sortBy("memMB")}>
      <span class="h-label">{$t("tm.col.memory")} {#if sortKey === "memMB"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}</span>
      <span class="h-total">{totalMem}</span>
    </button></span>
    <span class="hcell" role="columnheader" aria-sort={ariaSort("diskKBs")}><button class="h num" class:sorted={sortKey === "diskKBs"} aria-label={totalDisk ? $t("tm.col.withTotal", { col: $t("tm.col.disk"), total: totalDisk }) : $t("tm.col.disk")} onclick={() => sortBy("diskKBs")}>
      <span class="h-label">{$t("tm.col.disk")} {#if sortKey === "diskKBs"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}</span>
      <span class="h-total">{totalDisk}</span>
    </button></span>
    <span class="hcell" role="columnheader" aria-sort={ariaSort("netKBs")}><button class="h num" class:sorted={sortKey === "netKBs"} aria-label={totalNet ? $t("tm.col.withTotal", { col: $t("tm.col.network"), total: totalNet }) : $t("tm.col.network")} onclick={() => sortBy("netKBs")}>
      <span class="h-label">{$t("tm.col.network")} {#if sortKey === "netKBs"}<span class="arrow">{sortDir === "asc" ? "▲" : "▼"}</span>{/if}</span>
      <!-- The slot the other columns use for their live total says, for this one,
           that there is no total to give. Same place, so the eye reads it as an
           answer about the column rather than a stray label. -->
      <span class="h-total" class:unmeasured={!netMeasured}
        >{netMeasured ? totalNet : $t("tm.col.notMeasured")}</span>
    </button></span>
  </div>

  <div class="body">
    {#each items as it, i (it.kind === "group" ? `g-${it.id}` : `p-${it.proc.id}-${it.depth}`)}
      {#if it.kind === "group"}
        <div class="grouprow" role="presentation"><span>{$t(it.id)}</span></div>
      {:else}
        {@const p = it.proc}
        {@const sensors = sensorsFor(p.name)}
        <div
          class="row"
          class:child={it.depth > 0}
          class:selected={p.id === selectedId}
          role="row"
          aria-label={p.name}
          data-pid={p.id}
          data-critical={p.critical ? "1" : null}
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
                aria-label={$t("tm.row.expand")}
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
            <span>{$t(p.paused ? "tm.status.suspended" : STATUS_ID[p.status])}</span>
            {#if p.limited && !p.paused}<span class="limtag">{$t("tm.tag.limited")}</span>{/if}
          </div>
          <div class="cell access" role="gridcell" title={accessMeasured ? undefined : $t("tm.col.accessUnavailable")}>
            {#if !accessMeasured}<span class="unknown">-</span>{/if}
            {#if accessMeasured && sensors.camera}<Camera size={13} strokeWidth={2} />{/if}
            {#if accessMeasured && sensors.mic}<Mic size={13} strokeWidth={2} />{/if}
            {#if accessMeasured && sensors.knowledge}<span class="kg-glyph"><Brain size={13} strokeWidth={2} /></span>{/if}
          </div>
          <div class="cell num" role="gridcell" style="--heat: {dispHeat(p)}">{formatDecimal(dispCpu(p), 1, $locale)}%</div>
          <div class="cell num" role="gridcell" style="--heat: {heat(p.memMB, 2200)}">{mem(p.memMB)}</div>
          <div class="cell num muted" role="gridcell">{rate(p.diskKBs)}</div>
          <!-- Not a zero. Per-process network is not in /proc - it needs eBPF or
               cgroup attribution, and `procmon.rs` says so and reports 0 - so
               printing "0" here would state that this process used no network,
               which nobody measured. A dash says the column has no answer. -->
          <div class="cell num muted" role="gridcell" title={$t("tm.col.networkUnavailable")}
            >{netMeasured ? rate(p.netKBs) : "-"}</div>
        </div>
      {/if}
    {/each}
  </div>
</div>

<style>
  .pt {
    font-size: var(--text-sm);
  }
  .head,
  .row,
  .grouprow {
    display: grid;
    /* Every column but the name was a fixed rem width, and each of those numbers
       is a measurement of its ENGLISH header. In German the row falls apart:
       "SPEICHER" and "DATENTRÄGER" run together with no gap between two separate
       columns, and the "nicht gemessen" note under ZUGRIFF and NETZWERK wraps
       into its neighbour. `minmax(<the old width>, max-content)` keeps English
       pixel-identical and lets a longer word take the room it needs. Third one
       of these found on 17 August, after the timeline's verb column and
       ModuleCard's 84px - a fixed width around a translated word is the shape. */
    grid-template-columns:
      minmax(12rem, 1fr) minmax(8.5rem, max-content) minmax(4rem, max-content)
      minmax(5rem, max-content) minmax(6rem, max-content) minmax(6rem, max-content)
      minmax(6.5rem, max-content);
    align-items: center;
  }
  .head {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--color-bg-app, #0f0f0f);
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
  }
  /* The sortable headers are grid items styled by `.h`, but ARIA does not allow
     `role="columnheader"` on a <button> - axe flagged six of them. The role and
     `aria-sort` belong to the CELL, so each button now sits inside one; the cell
     is `display: contents` so the button remains the grid item and the layout is
     byte-for-byte what it was. Verified with a screenshot either side, and with
     axe, which still sees the columnheaders through the contents box. */
  .hcell {
    display: contents;
  }

  .h {
    display: flex;
    align-items: baseline;
    gap: 0.3rem;
    padding: 0.5rem 0.6rem;
    border: none;
    background: transparent;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    /* 50, not 45. Measured against the rendered page rather than picked: the
       header is `--color-fg-primary` over `rgb(15,15,15)`, and at 45% that is
       #797979 for a contrast of 4.40 - under the 4.5 floor for text this size,
       which axe reported on all nine headers. 50% is the smallest step that
       clears it, at 5.13, and the difference on screen is 121 grey against 132. */
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
    cursor: pointer;
    text-align: start;
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
  .cell.access .unknown {
    /* Not the cell's warning amber, which it would inherit: amber on a dash reads
       as a caution about the process, and this dash is a statement about the
       column. The muted foreground says "no answer" without saying anything about
       the row it sits in. */
    color: var(--color-fg-primary, #fafafa);
    opacity: 0.4;
  }
  .h-total.unmeasured {
    /* Quieter than a number, because it is the absence of one. */
    opacity: 0.55;
    font-style: italic;
  }
  .h-label {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
  }
  .h-total {
    font-size: var(--text-2xs);
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .arrow {
    font-size: var(--text-2xs);
  }

  .grouprow {
    padding: 0.55rem 0.6rem 0.25rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    /* 50 for the same measured reason as the column headers above: 40% is
       #6b6b6b on this background, a ratio of 3.70, and this text is 12px. */
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
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
    font-size: var(--text-2xs);
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
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .limtag {
    padding: 0.02rem 0.3rem;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 10%, transparent);
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--color-fg-primary) 60%, transparent);
  }
  .cell.status[data-status="not-responding"] {
    color: var(--color-warning, #d0a54a);
  }
  .cell.status[data-status="suspended"] {
    /* 50 for the same measured reason as the column headers above: 40% is
       #6b6b6b on this background, a ratio of 3.70, and this text is 12px. */
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .cell.num {
    text-align: end;
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
