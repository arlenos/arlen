<script lang="ts">
  /// Whether the access block reflects the machine. It does not yet: see the
  /// comment at its render site, and the matching flag in `ProcessTable`. Both
  /// flip together the day the permission profile drives this.
  /// What the selected process is holding, read from `/proc/<pid>/fd` on
  /// demand. `undefined` until the first answer arrives; a field inside it
  /// being undefined means the read was refused, which the pane says rather
  /// than smoothing into an empty list.
  let held = $state<HeldResources | undefined>(undefined);

  /// The Statistics and Memory figures, read alongside the fd table.
  let stats = $state<ProcStats | undefined>(undefined);

  $effect(() => {
    const pid = process.id;
    held = undefined;
    stats = undefined;
    statsFor(pid).then((s) => {
      if (process.id === pid) stats = s;
    });
    // The pid is captured, so an answer for a row the user has already left is
    // discarded instead of being painted under the new selection.
    heldFor(pid).then((h) => {
      if (process.id === pid) held = h;
    });
  });

  /// The sensors are measured exactly when the fd table could be read. A process
  /// belonging to another user leaves this false, and the pane keeps saying so.
  ///
  /// `!= null` and not `!== undefined`: serde writes a Rust `None` as JSON
  /// `null`, so the strict check passed for a process whose fd table had been
  /// REFUSED, and the pane went back to printing "Not using your camera,
  /// microphone, or screen" about a process it had never read. That false
  /// clearance is the defect this whole pane exists to remove, and I put it back
  /// in for twenty minutes by trusting a type name over the wire.
  const accessMeasured = $derived(held?.camera != null);

  import { t, locale } from "$lib/i18n/messages";
  import { formatDecimal } from "@arlen/ui-kit/i18n";
  /// The per-process detail pane. Standard tabs (Statistics / Memory / Open files)
  /// plus the Arlen-native ACCESS tab: what the process holds + the KG capability
  /// scopes it holds, revocable right here. The sovereign angle as per-process
  /// detail, not a landing.
  import { detailFor, heldFor, statsFor, type HeldResources, type ProcDetail, type ProcStats } from "$lib/stores/detail";
  import type { Process } from "$lib/stores/processes";
  import { ScopeChip } from "@arlen/ui-kit/components/ui/scope-chip";
  import { X, Camera, Mic, Cog, Cpu } from "lucide-svelte";

  let {
    process,
    onClose,
    onForceQuit,
  }: { process: Process; onClose: () => void; onForceQuit: (id: number) => void } = $props();

  const detail = $derived<ProcDetail>(detailFor(process));
  let confirmQuit = $state(false);
  const TABS = ["Access", "Statistics", "Memory", "Open files"] as const;
  let tab = $state<(typeof TABS)[number]>("Access");

  // Reset the quit confirm when the selected process changes.
  $effect(() => {
    process;
    confirmQuit = false;
  });
  function forceQuit() {
    if (!confirmQuit) {
      confirmQuit = true;
      return;
    }
    onForceQuit(process.id);
  }

  /// An em-dash-free placeholder for a figure that was not measured. Printing a
  /// zero, or a number derived from the row, is what this pane did before: the
  /// thread count was memory divided by 40, so it moved when memory did and read
  /// like a measurement.
  const UNMEASURED = "-";
  function num(v: number | null | undefined): string {
    return v == null ? UNMEASURED : formatDecimal(v, 0, $locale);
  }
  /// A pid is an identifier, not a quantity: grouping it reads as "2,965,880"
  /// beside a Process ID printed plainly, and nobody groups a pid anywhere else
  /// in the tool.
  function id(v: number | null | undefined): string {
    return v == null ? UNMEASURED : String(v);
  }
  const STATE_ROWS = $derived([
    ["Process ID", String(process.id)],
    ["Parent process", id(stats?.ppid)],
    ["Threads", num(stats?.threads)],
    ["State", stats?.state ?? UNMEASURED],
    ["Priority", num(stats?.nice)],
    ["Context switches", num(stats?.ctxSwitches)],
  ]);
  function mem(mb: number): string {
    return mb >= 1024
      ? `${formatDecimal(mb / 1024, 1, $locale)} GB`
      : `${formatDecimal(mb, 0, $locale)} MB`;
  }
</script>

<aside class="dp">
  <header class="dp-head">
    <span class="dp-icon" aria-hidden="true">
      {#if process.group === "app"}{process.name.charAt(0)}
      {:else if process.group === "background"}<Cog size={13} strokeWidth={2} />
      {:else}<Cpu size={13} strokeWidth={2} />{/if}
    </span>
    <div class="dp-id">
      <span class="dp-name">{process.name}</span>
      <span class="dp-pid">{$t("tm.dp.pid", { pid: detail.pid })}</span>
    </div>
    <button
      type="button"
      class="dp-quit"
      class:confirm={confirmQuit}
      onclick={forceQuit}
      onblur={() => (confirmQuit = false)}
    >
      {confirmQuit ? $t("tm.dp.forceQuitConfirm") : $t("tm.dp.forceQuit")}
    </button>
    <button type="button" class="dp-close" aria-label={$t("tm.dp.close")} onclick={onClose}><X size={15} strokeWidth={2} /></button>
  </header>

  <nav class="dp-tabs">
    {#each TABS as t (t)}
      <button type="button" class="dp-tab" class:active={tab === t} onclick={() => (tab = t)}>{t}</button>
    {/each}
  </nav>

  <div class="dp-body">
    {#if tab === "Access"}
      <div class="acc-sensor" data-lit={held?.camera || held?.mic}>
        <!-- MEASURED, OR SAID TO BE UNMEASURED - never a reassurance nobody checked.
             `detail.access` comes from a hand-keyed table in `stores/detail.ts`
             matched on process name, which the file itself calls a stand-in. For
             any process not in that table it falls to a default that reads "Not
             using your camera, microphone, or screen" and "It runs with limited
             access and holds nothing sensitive" - a confident clearance, printed
             about a process nobody looked at. Found on 16 August by opening the
             pane on `claude` and reading what it said. -->
        {#if !accessMeasured}
          <span>{$t("tm.dp.accessUnknown")}</span>
        {:else if held?.camera || held?.mic}
          {#if held?.camera}<Camera size={15} strokeWidth={2} />{/if}
          {#if held?.mic}<Mic size={15} strokeWidth={2} />{/if}
          <span>{held?.camera && held?.mic
            ? $t("tm.dp.usingBoth")
            : held?.camera
              ? $t("tm.dp.usingCamera")
              : $t("tm.dp.usingMic")}</span>
        {:else}
          <span>{$t("tm.dp.usingNothing")}</span>
        {/if}
      </div>

      <p class="acc-reach">{accessMeasured ? detail.access.reach : $t("tm.dp.reachUnknown")}</p>

      {#if detail.access.scopes.length > 0}
        <div class="acc-scopes">
          <h3 class="acc-h">{$t("tm.dp.knowledgeAccess")}</h3>
          <div class="acc-chips">
            {#each detail.access.scopes as s (s.label)}
              <ScopeChip label={s.label} />
            {/each}
          </div>
        </div>
      {/if}

      <button type="button" class="acc-manage" onclick={() => {}}>{$t("tm.dp.manageInAppAccess")}</button>
    {:else if tab === "Statistics"}
      <dl class="stats">
        {#each STATE_ROWS as [k, v] (k)}
          <div class="stat"><dt>{k}</dt><dd>{v}</dd></div>
        {/each}
      </dl>
    {:else if tab === "Memory"}
      <!-- `smaps_rollup` is owner-readable only, so another user's process has no
           PSS to show. It stays blank rather than borrowing RSS: the two answer
           different questions and the gap grows with sharing, so a browser would
           be misreported by hundreds of megabytes under a label saying PSS. -->
      <dl class="stats">
        <div class="stat"><dt>{$t("tm.dp.rss")}</dt><dd>{stats?.rssMB == null ? UNMEASURED : mem(stats.rssMB)}</dd></div>
        <div class="stat"><dt>{$t("tm.dp.pss")}</dt><dd>{stats?.pssMB == null ? UNMEASURED : mem(stats.pssMB)}</dd></div>
        <div class="stat"><dt>{$t("tm.dp.shared")}</dt><dd>{stats?.sharedMB == null ? UNMEASURED : mem(stats.sharedMB)}</dd></div>
      </dl>
      {#if stats?.unreadable}<p class="empty">{stats.unreadable}</p>{/if}
    {:else}
      <div class="files">
        <!-- Real, or said to be unread. The invented version built three paths
             from the process name and gave anything with traffic a hardcoded
             GitHub address, so the pane showed a connection for a process it had
             never inspected. `undefined` here means the fd table was refused
             (another user's process), which is NOT the same as holding nothing
             open and must not render as "no open files". -->
        {#if held === undefined}
          <p class="empty">{$t("tm.dp.readingFiles")}</p>
        {:else if held.openFiles == null}
          <p class="empty">{held.unreadable ?? $t("tm.dp.filesUnknown")}</p>
        {:else}
          {#each held.openFiles as f (f)}<div class="fline">{f}</div>{/each}
          {#each held.connections ?? [] as c (c.proto + c.local + c.peer)}
            <div class="fline conn">{c.proto} {c.local} → {c.peer} {c.state}</div>
          {/each}
          {#if held.openFiles.length === 0 && (held.connections ?? []).length === 0}
            <p class="empty">{$t("tm.dp.noOpenFiles")}</p>
          {/if}
        {/if}
      </div>
    {/if}
  </div>
</aside>

<style>
  .dp {
    width: 22rem;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    border-inline-start: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    overflow: hidden;
  }
  .dp-head {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    padding: 0.85rem 0.9rem;
  }
  .dp-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.5rem;
    height: 1.5rem;
    flex-shrink: 0;
    border-radius: var(--radius-chip, 4px);
    background: color-mix(in srgb, var(--color-fg-primary) 12%, transparent);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--color-fg-primary);
  }
  .dp-id {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .dp-name {
    font-size: var(--text-md);
    font-weight: 600;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dp-pid {
    font-size: var(--text-2xs);
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .dp-quit {
    flex-shrink: 0;
    padding: 0.3rem 0.6rem;
    border: 1px solid color-mix(in srgb, var(--color-error, #c96a6a) 40%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font-size: var(--text-xs);
    color: var(--color-error, #c96a6a);
  }
  .dp-quit.confirm {
    background: color-mix(in srgb, var(--color-error, #c96a6a) 16%, transparent);
  }
  .dp-close {
    flex-shrink: 0;
    display: inline-flex;
    padding: 0.2rem;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .dp-close:hover {
    color: var(--color-fg-primary);
  }

  .dp-tabs {
    display: flex;
    gap: 0.15rem;
    padding: 0 0.7rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .dp-tab {
    position: relative;
    padding: 0.5rem 0.55rem;
    border: none;
    background: transparent;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .dp-tab:hover {
    color: var(--color-fg-primary);
  }
  .dp-tab.active {
    color: var(--color-fg-primary);
  }
  .dp-tab.active::after {
    content: "";
    position: absolute;
    inset-inline: 0.55rem;
    bottom: -1px;
    height: 2px;
    background: var(--color-fg-primary);
  }

  .dp-body {
    flex: 1;
    overflow-y: auto;
    padding: 1rem 0.9rem;
  }

  .acc-sensor {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.55rem 0.65rem;
    border-radius: var(--radius-input, 8px);
    background: color-mix(in srgb, var(--color-fg-primary) 5%, transparent);
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 65%, transparent);
  }
  .acc-sensor[data-lit="true"] {
    background: color-mix(in srgb, var(--color-warning, #d0a54a) 14%, transparent);
    color: var(--color-warning, #d0a54a);
  }
  .acc-reach {
    margin: 0.9rem 0 0;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 78%, transparent);
  }
  .acc-scopes {
    margin-top: 1.1rem;
  }
  .acc-h {
    margin: 0 0 0.5rem;
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .acc-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
  }
  .acc-manage {
    margin-top: 1.25rem;
    padding: 0.35rem 0.7rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 15%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font-size: var(--text-sm);
    color: var(--color-fg-primary);
  }
  .acc-manage:hover {
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }

  .stats {
    margin: 0;
    display: flex;
    flex-direction: column;
  }
  .stat {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.5rem 0;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
    font-size: var(--text-sm);
  }
  .stat dt {
    color: color-mix(in srgb, var(--color-fg-primary) 55%, transparent);
  }
  .stat dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
    color: var(--color-fg-primary);
  }

  .files {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-xs);
  }
  .fline {
    padding: 0.25rem 0;
    color: color-mix(in srgb, var(--color-fg-primary) 70%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .fline.conn {
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
  }
  .empty {
    margin: 0;
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
</style>
