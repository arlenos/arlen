<script lang="ts">
  /// Card showing a single discovered module.
  /// Expandable: collapsed shows icon + name + toggle; expanded shows
  /// description, badges, warnings and the uninstall button.

  import {
    ChevronRight,
    Trash2,
    Puzzle,
    Search,
    LayoutDashboard,
    Settings2,
    AlertTriangle,
  } from "lucide-svelte";
  import { Switch } from "$lib/components/ui/switch";
  import type { ModuleSummary } from "$lib/stores/modules";

  let {
    module,
    onToggle,
    onUninstall,
  }: {
    module: ModuleSummary;
    onToggle: (enabled: boolean) => void;
    onUninstall: () => void;
  } = $props();

  let expanded = $state(false);
  let confirmingUninstall = $state(false);

  function clickUninstall() {
    if (!confirmingUninstall) {
      confirmingUninstall = true;
      setTimeout(() => (confirmingUninstall = false), 4000);
      return;
    }
    confirmingUninstall = false;
    onUninstall();
  }
</script>

<div class="card" class:expanded class:disabled={!module.enabled}>
  <div class="head">
    <button
      type="button"
      class="head-toggle"
      aria-expanded={expanded}
      onclick={() => (expanded = !expanded)}
    >
      <ChevronRight size={12} strokeWidth={2.5} class="card-chev" />
      <div class="icon-wrap">
        <Puzzle size={14} strokeWidth={2} class="mod-icon" />
      </div>
      <div class="titles">
        <span class="name">{module.name || module.id}</span>
        <span class="meta">
          <span class="version">v{module.version || "?"}</span>
          {#if module.source === "system"}
            <span class="badge system">System</span>
          {:else}
            <span class="badge">User</span>
          {/if}
          {#if module.hasWaypointer}
            <span class="badge ext"><Search size={9} strokeWidth={2.5} /> Waypointer</span>
          {/if}
          {#if module.hasTopbar}
            <span class="badge ext"><LayoutDashboard size={9} strokeWidth={2.5} /> TopBar</span>
          {/if}
          {#if module.hasSettings}
            <span class="badge ext"><Settings2 size={9} strokeWidth={2.5} /> Settings</span>
          {/if}
        </span>
      </div>
    </button>
    <div class="head-right">
      <Switch
        value={module.enabled}
        onchange={onToggle}
        ariaLabel={module.enabled ? "Disable module" : "Enable module"}
      />
    </div>
  </div>

  {#if expanded}
    <div class="body">
      {#if module.description}
        <p class="desc">{module.description}</p>
      {:else}
        <p class="desc muted">No description provided.</p>
      {/if}

      <div class="rows">
        <div class="row">
          <span class="row-label">ID</span>
          <span class="row-value mono">{module.id}</span>
        </div>
        <div class="row">
          <span class="row-label">Trust tier</span>
          <span class="row-value">{module.moduleType}</span>
        </div>
        <div class="row">
          <span class="row-label">Install path</span>
          <span class="row-value mono path" title={module.path}>{module.path}</span>
        </div>
      </div>

      {#if module.warnings.length > 0}
        <div class="warnings">
          <div class="warnings-head">
            <AlertTriangle size={12} strokeWidth={2.25} />
            <span>Manifest warnings</span>
          </div>
          <ul>
            {#each module.warnings as w}
              <li>{w}</li>
            {/each}
          </ul>
        </div>
      {/if}

      {#if module.source === "user"}
        <div class="actions">
          <button
            type="button"
            class="danger-btn"
            class:confirming={confirmingUninstall}
            onclick={clickUninstall}
          >
            <Trash2 size={12} strokeWidth={2.25} />
            {confirmingUninstall ? "Confirm — click again" : "Uninstall"}
          </button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .card {
    border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    overflow: hidden;
    transition: opacity 150ms ease;
  }
  .card.disabled {
    opacity: 0.55;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0.5rem 0.625rem 0.5rem 0;
  }
  .head-toggle {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 0;
    padding: 0.25rem 0.5rem 0.25rem 0.6rem;
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    color: var(--foreground);
    font: inherit;
    border-radius: var(--radius-sm);
    transition: background-color 120ms ease;
  }
  .head-toggle:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }

  :global(.card-chev) {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    transition: transform 150ms ease;
    flex-shrink: 0;
  }
  .card.expanded :global(.card-chev) {
    transform: rotate(90deg);
  }

  .icon-wrap {
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex-shrink: 0;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-accent) 15%, transparent);
    color: var(--color-accent);
  }
  :global(.mod-icon) {
    color: inherit;
  }

  .titles {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .name {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    font-size: 0.625rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .version {
    font-variant-numeric: tabular-nums;
  }
  .badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 0.05rem 0.35rem;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    font-weight: 600;
    letter-spacing: 0.02em;
    text-transform: uppercase;
  }
  .badge.system {
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    color: var(--color-accent);
  }
  .badge.ext {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    text-transform: none;
    letter-spacing: 0;
    font-weight: 500;
  }

  .head-right {
    flex-shrink: 0;
  }

  .body {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    padding: 0.6rem 0.7rem 0.7rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }

  .desc {
    margin: 0;
    font-size: 0.75rem;
    line-height: 1.45;
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  .desc.muted {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    font-style: italic;
  }

  .rows {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .row {
    display: flex;
    gap: 10px;
    font-size: 0.6875rem;
  }
  .row-label {
    width: 84px;
    flex-shrink: 0;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .row-value {
    flex: 1;
    min-width: 0;
    color: color-mix(in srgb, var(--foreground) 85%, transparent);
  }
  .mono {
    font-family: var(--font-mono);
    font-size: 0.625rem;
  }
  .path {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .warnings {
    padding: 0.5rem 0.6rem;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-warning) 10%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-warning) 30%, transparent);
    color: var(--color-warning);
    font-size: 0.6875rem;
  }
  .warnings-head {
    display: flex;
    align-items: center;
    gap: 5px;
    font-weight: 600;
    margin-bottom: 4px;
  }
  .warnings ul {
    margin: 0;
    padding-left: 1rem;
    color: color-mix(in srgb, var(--color-warning) 85%, transparent);
  }

  .actions {
    display: flex;
    justify-content: flex-start;
  }
  .danger-btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0.35rem 0.6rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-error) 35%, transparent);
    color: var(--color-error);
    font-size: 0.6875rem;
    cursor: pointer;
    transition: background-color 120ms ease;
  }
  .danger-btn:hover {
    background: color-mix(in srgb, var(--color-error) 18%, transparent);
  }
  .danger-btn.confirming {
    background: color-mix(in srgb, var(--color-error) 28%, transparent);
    border-color: var(--color-error);
  }
</style>
