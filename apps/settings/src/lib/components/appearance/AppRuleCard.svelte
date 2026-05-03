<script lang="ts">
  /// Per-app notification rule card. Expandable row that exposes the
  /// four daemon-supported overrides (`enabled`, `suppress`,
  /// `bypass_dnd`, `priority`) plus an optional toast duration.

  import { ChevronRight, X } from "lucide-svelte";
  import { Switch } from "$lib/components/ui/switch";
  import type { AppOverride } from "$lib/stores/notifications";

  let {
    appName,
    rule,
    onchange,
    onremove,
  }: {
    appName: string;
    rule: AppOverride;
    onchange: (patch: Partial<AppOverride>) => void;
    onremove: () => void;
  } = $props();

  let expanded = $state(false);

  const PRIORITIES = ["low", "normal", "high", "critical"] as const;

  // Helper: a rule is "active" if any field deviates from defaults.
  const isCustomised = $derived(
    rule.enabled === false ||
      rule.suppress === true ||
      rule.bypass_dnd === true ||
      rule.priority !== undefined ||
      rule.toast_duration !== undefined
  );
</script>

<div class="card" class:expanded>
  <div class="head">
    <button
      type="button"
      class="head-toggle"
      aria-expanded={expanded}
      onclick={() => (expanded = !expanded)}
    >
      <ChevronRight size={12} strokeWidth={2.5} class="card-chev" />
      <span class="title">{appName}</span>
      {#if isCustomised}
        <span class="badge">Custom</span>
      {/if}
      {#if rule.enabled === false}
        <span class="badge muted">Blocked</span>
      {:else if rule.suppress === true}
        <span class="badge muted">Silent</span>
      {/if}
    </button>
    <button
      type="button"
      class="remove"
      title="Remove rule"
      aria-label="Remove rule"
      onclick={onremove}
    >
      <X size={12} strokeWidth={2.5} />
    </button>
  </div>

  {#if expanded}
    <div class="body">
      <div class="row">
        <span class="row-label">
          Notifications enabled
          <span class="row-hint">Off blocks the app entirely.</span>
        </span>
        <Switch
          value={rule.enabled !== false}
          onchange={(v) => onchange({ enabled: v ? undefined : false })}
          ariaLabel="Notifications enabled"
        />
      </div>

      <div class="row">
        <span class="row-label">
          Silent
          <span class="row-hint">Stored in history but no toast.</span>
        </span>
        <Switch
          value={rule.suppress === true}
          onchange={(v) => onchange({ suppress: v ? true : undefined })}
          ariaLabel="Silent"
        />
      </div>

      <div class="row">
        <span class="row-label">
          Bypass Do Not Disturb
          <span class="row-hint">Notifications break through any DND mode.</span>
        </span>
        <Switch
          value={rule.bypass_dnd === true}
          onchange={(v) => onchange({ bypass_dnd: v ? true : undefined })}
          ariaLabel="Bypass DND"
        />
      </div>

      <div class="row">
        <span class="row-label">
          Force priority
          <span class="row-hint">Override the priority the app reports.</span>
        </span>
        <div class="prio-pills">
          <button
            type="button"
            class="prio-pill"
            class:selected={rule.priority === undefined}
            onclick={() => onchange({ priority: undefined })}
          >
            Auto
          </button>
          {#each PRIORITIES as p}
            <button
              type="button"
              class="prio-pill"
              class:selected={rule.priority === p}
              onclick={() => onchange({ priority: p })}
            >
              {p}
            </button>
          {/each}
        </div>
      </div>
    </div>
  {/if}
</div>

<style>
  .card {
    border: 1px solid color-mix(in srgb, var(--foreground) 9%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    overflow: hidden;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    padding: 0 0.4rem 0 0;
    color: var(--foreground);
  }
  .head-toggle {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
    height: 36px;
    padding: 0 0.5rem 0 0.6rem;
    background: transparent;
    border: none;
    cursor: pointer;
    text-align: left;
    color: inherit;
    font: inherit;
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

  .title {
    font-size: 0.8125rem;
    font-weight: 500;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .badge {
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    color: var(--color-accent);
  }
  .badge.muted {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    color: color-mix(in srgb, var(--foreground) 65%, transparent);
  }

  .remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-chip);
    background: transparent;
    border: none;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    cursor: pointer;
    flex-shrink: 0;
    transition:
      color 120ms ease,
      background-color 120ms ease;
  }
  .remove:hover {
    color: var(--color-error);
    background: color-mix(in srgb, var(--color-error) 12%, transparent);
  }

  .body {
    border-top: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    padding: 0.5rem 0.6rem 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 1rem;
    min-height: 30px;
  }
  .row-label {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
    font-size: 0.75rem;
    font-weight: 500;
  }
  .row-hint {
    font-size: 0.6875rem;
    font-weight: 400;
    color: color-mix(in srgb, var(--foreground) 48%, transparent);
  }

  .prio-pills {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .prio-pill {
    height: 22px;
    padding: 0 0.55rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    font-size: 0.6875rem;
    font-weight: 500;
    cursor: pointer;
    text-transform: capitalize;
    transition:
      background-color 120ms ease,
      border-color 120ms ease,
      color 120ms ease;
  }
  .prio-pill:hover {
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
    color: var(--foreground);
  }
  .prio-pill.selected {
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    border-color: color-mix(in srgb, var(--color-accent) 35%, transparent);
    color: var(--foreground);
  }
</style>
