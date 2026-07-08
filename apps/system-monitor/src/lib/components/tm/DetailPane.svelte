<script lang="ts">
  /// The per-process detail pane. Standard tabs (Statistics / Memory / Open files)
  /// plus the Arlen-native ACCESS tab: what the process holds + the KG capability
  /// scopes it holds, revocable right here. The sovereign angle as per-process
  /// detail, not a landing.
  import { detailFor, revokeScope, type ProcDetail } from "$lib/stores/detail";
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
  let revoked = $state<Set<string>>(new Set());
  const visibleScopes = $derived(detail.access.scopes.filter((s) => !revoked.has(s.label)));
  const TABS = ["Access", "Statistics", "Memory", "Open files"] as const;
  let tab = $state<(typeof TABS)[number]>("Access");

  // Reset the revoke set + the quit confirm when the selected process changes.
  $effect(() => {
    process;
    confirmQuit = false;
    revoked = new Set();
  });

  function revoke(label: string) {
    revoked = new Set(revoked).add(label);
    void revokeScope(process.id, label);
  }
  function forceQuit() {
    if (!confirmQuit) {
      confirmQuit = true;
      return;
    }
    onForceQuit(process.id);
  }

  const STATE_ROWS = $derived([
    ["Process ID", String(detail.pid)],
    ["Parent process", String(detail.ppid)],
    ["Threads", String(detail.threads)],
    ["State", detail.state],
    ["Priority", String(detail.priority)],
    ["Context switches", detail.ctxSwitches.toLocaleString()],
  ]);
  function mem(mb: number): string {
    return mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${mb} MB`;
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
      <span class="dp-pid">PID {detail.pid}</span>
    </div>
    <button
      type="button"
      class="dp-quit"
      class:confirm={confirmQuit}
      onclick={forceQuit}
      onblur={() => (confirmQuit = false)}
    >
      {confirmQuit ? "Force quit?" : "Force Quit"}
    </button>
    <button type="button" class="dp-close" aria-label="Close" onclick={onClose}><X size={15} strokeWidth={2} /></button>
  </header>

  <nav class="dp-tabs">
    {#each TABS as t (t)}
      <button type="button" class="dp-tab" class:active={tab === t} onclick={() => (tab = t)}>{t}</button>
    {/each}
  </nav>

  <div class="dp-body">
    {#if tab === "Access"}
      <div class="acc-sensor" data-lit={detail.access.camera || detail.access.mic}>
        {#if detail.access.camera || detail.access.mic}
          {#if detail.access.camera}<Camera size={15} strokeWidth={2} />{/if}
          {#if detail.access.mic}<Mic size={15} strokeWidth={2} />{/if}
          <span>Using your {[detail.access.camera && "camera", detail.access.mic && "microphone"].filter(Boolean).join(" and ")} right now.</span>
        {:else}
          <span>Not using your camera, microphone, or screen.</span>
        {/if}
      </div>

      <p class="acc-reach">{detail.access.reach}</p>

      {#if visibleScopes.length > 0}
        <div class="acc-scopes">
          <h3 class="acc-h">Knowledge access</h3>
          <div class="acc-chips">
            {#each visibleScopes as s (s.label)}
              <ScopeChip label={s.label} onRevoke={() => revoke(s.label)} />
            {/each}
          </div>
        </div>
      {/if}

      <button type="button" class="acc-manage" onclick={() => {}}>Manage in App access</button>
    {:else if tab === "Statistics"}
      <dl class="stats">
        {#each STATE_ROWS as [k, v] (k)}
          <div class="stat"><dt>{k}</dt><dd>{v}</dd></div>
        {/each}
      </dl>
    {:else if tab === "Memory"}
      <dl class="stats">
        <div class="stat"><dt>Resident (RSS)</dt><dd>{mem(detail.rssMB)}</dd></div>
        <div class="stat"><dt>Proportional (PSS)</dt><dd>{mem(detail.pssMB)}</dd></div>
        <div class="stat"><dt>Shared</dt><dd>{mem(detail.sharedMB)}</dd></div>
      </dl>
    {:else}
      <div class="files">
        {#each detail.openFiles as f (f)}<div class="fline">{f}</div>{/each}
        {#each detail.connections as c (c)}<div class="fline conn">{c}</div>{/each}
        {#if detail.openFiles.length === 0 && detail.connections.length === 0}
          <p class="empty">No open files or connections.</p>
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
    border-left: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
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
    font-size: 0.75rem;
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
    font-size: 0.9375rem;
    font-weight: 600;
    color: var(--color-fg-primary);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dp-pid {
    font-size: 0.6875rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
  .dp-quit {
    flex-shrink: 0;
    padding: 0.3rem 0.6rem;
    border: 1px solid color-mix(in srgb, var(--color-error, #c96a6a) 40%, transparent);
    border-radius: var(--radius-input, 8px);
    background: transparent;
    font-size: 0.75rem;
    color: var(--color-error, #c96a6a);
    cursor: pointer;
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
    cursor: pointer;
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
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--color-fg-primary) 50%, transparent);
    cursor: pointer;
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
    left: 0.55rem;
    right: 0.55rem;
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
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-primary) 65%, transparent);
  }
  .acc-sensor[data-lit="true"] {
    background: color-mix(in srgb, var(--color-warning, #d0a54a) 14%, transparent);
    color: var(--color-warning, #d0a54a);
  }
  .acc-reach {
    margin: 0.9rem 0 0;
    font-size: 0.8125rem;
    line-height: 1.5;
    color: color-mix(in srgb, var(--color-fg-primary) 78%, transparent);
  }
  .acc-scopes {
    margin-top: 1.1rem;
  }
  .acc-h {
    margin: 0 0 0.5rem;
    font-size: 0.6875rem;
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
    font-size: 0.8125rem;
    color: var(--color-fg-primary);
    cursor: pointer;
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
    font-size: 0.8125rem;
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
    font-size: 0.75rem;
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
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
</style>
