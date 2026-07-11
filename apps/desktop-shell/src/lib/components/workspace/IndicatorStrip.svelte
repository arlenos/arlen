<script lang="ts">
  /// The topbar workspace strip. Three densities, picked by the
  /// host from the workspace count: pills (≤5), dots (≤9), and a
  /// plain "n / total" text readout beyond that.

  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import { pillLabel, fullLabel } from "$lib/workspace/format.js";

  let {
    workspaces,
    mode,
    activeIndex,
    onActivate,
  }: {
    workspaces: WorkspaceInfo[];
    mode: "pills" | "dots" | "text";
    /// Index of the active workspace, -1 when none is flagged.
    activeIndex: number;
    onActivate: (id: string) => void;
  } = $props();
</script>

{#if mode === "pills"}
  <div class="indicator" role="group" aria-label="Workspaces">
    {#each workspaces as ws, i (ws.id)}
      <button
        class="pill"
        class:pill-active={ws.active}
        onclick={() => onActivate(ws.id)}
        aria-label={fullLabel(ws, i)}
        aria-pressed={ws.active}
      >
        {pillLabel(ws, i)}
      </button>
    {/each}
  </div>
{:else if mode === "dots"}
  <div class="indicator" role="group" aria-label="Workspaces">
    {#each workspaces as ws, i (ws.id)}
      <button
        class="dot-btn"
        onclick={() => onActivate(ws.id)}
        aria-label={fullLabel(ws, i)}
        aria-pressed={ws.active}
      >
        <span class="dot" class:dot-active={ws.active}></span>
      </button>
    {/each}
  </div>
{:else}
  <div class="indicator" role="group" aria-label="Workspaces">
    <span class="ws-text">
      {activeIndex >= 0 ? activeIndex + 1 : 1} / {workspaces.length}
    </span>
  </div>
{/if}

<style>
  .indicator {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  /* ── Pills ──────────────────────────────────────────────────────────── */

  .pill {
    display: flex;
    align-items: center;
    justify-content: center;
    height: var(--height-control-compact, 24px);
    min-width: 32px;
    padding: 0 10px;
    border-radius: var(--radius-card);
    border: none;
    font-size: var(--text-2xs);
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
    transition:
      background-color var(--duration-fast, 150ms) ease,
      color var(--duration-fast, 150ms) ease,
      transform var(--duration-micro, 100ms) ease;
    background: transparent;
    color: var(--foreground);
  }

  .pill:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }

  .pill:active {
    transform: scale(0.95);
    transition: transform 50ms ease;
  }

  .pill-active {
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    color: var(--color-accent);
    animation: pill-activate var(--duration-micro, 100ms) ease forwards;
  }

  .pill-active:hover {
    background: color-mix(in srgb, var(--color-accent) 26%, transparent);
  }

  @keyframes pill-activate {
    from {
      transform: scale(0.9);
    }
    to {
      transform: scale(1);
    }
  }

  /* ── Dots ───────────────────────────────────────────────────────────── */

  .dot-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    padding: 0;
    border: none;
    background: transparent;
    border-radius: var(--radius-full);
    transition: transform var(--duration-micro, 100ms) ease;
  }

  .dot-btn:active {
    transform: scale(0.85);
  }

  .dot {
    display: block;
    width: 5px;
    height: 5px;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 45%, transparent);
    transition:
      width var(--duration-micro, 100ms) ease,
      height var(--duration-micro, 100ms) ease,
      background-color var(--duration-fast, 150ms) ease;
  }

  .dot-btn:hover .dot {
    background: color-mix(in srgb, var(--foreground) 70%, transparent);
  }

  .dot-active {
    width: 7px;
    height: 7px;
    background: var(--color-accent);
    animation: dot-activate var(--duration-micro, 100ms) ease forwards;
  }

  .dot-btn:hover .dot-active {
    background: color-mix(
      in srgb,
      var(--color-accent) 85%,
      var(--color-fg-shell) 15%
    );
  }

  @keyframes dot-activate {
    from {
      transform: scale(0.7);
    }
    to {
      transform: scale(1);
    }
  }

  /* ── Text ───────────────────────────────────────────────────────────── */

  .ws-text {
    font-size: var(--text-2xs);
    font-weight: 500;
    color: var(--foreground);
    letter-spacing: 0.02em;
  }
</style>
