<script lang="ts">
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import { workspaces, activateWorkspace, type WorkspaceInfo } from "$lib/stores/workspaces.js";

  /// Adaptive display mode derived from workspace count.
  const mode = $derived(
    $workspaces.length <= 5 ? "pills" :
    $workspaces.length <= 9 ? "dots"  : "text"
  );

  const activeIndex = $derived($workspaces.findIndex((w) => w.active));

  // Shell-surface colors used explicitly because the Portal renders into
  // document.body (outside the .shell-surface CSS context). Using bg-popover
  // + text-popover-foreground gives dark-on-dark which is unreadable.
  const tooltipClass =
    "rounded-md border px-2 py-0.5 text-xs shadow-md select-none"
    + " bg-[var(--color-bg-shell)] text-[var(--color-fg-shell)] border-[color-mix(in_srgb,var(--color-bg-shell)_60%,white_40%)]";

  /// Display label for a pill: the workspace name (truncated) or its 1-based number.
  function pillLabel(ws: WorkspaceInfo, i: number): string {
    const name = ws.name.trim();
    if (!name) return String(i + 1);
    return name.length > 12 ? name.slice(0, 12) + "…" : name;
  }

  /// Full name for tooltip / aria-label.
  function fullLabel(ws: WorkspaceInfo, i: number): string {
    return ws.name.trim() || `Workspace ${i + 1}`;
  }

  function handleClick(id: string) {
    activateWorkspace(id);
  }
</script>

{#if $workspaces.length > 0}
  {#if mode === "pills"}
    <!--
      Pills mode (1-5 workspaces): each workspace is a rounded button.
      Active pill uses var(--accent) fill. Inactive pills are ghost-style.
    -->
    <div class="indicator" role="group" aria-label="Workspaces">
      {#each $workspaces as ws, i}
        <Tooltip.Root instant>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <button
                class="pill"
                class:pill-active={ws.active}
                onclick={() => handleClick(ws.id)}
                aria-label={fullLabel(ws, i)}
                aria-pressed={ws.active}
                {...props}
              >
                {pillLabel(ws, i)}
              </button>
            {/snippet}
          </Tooltip.Trigger>
          <Tooltip.Portal>
            <Tooltip.Content side="bottom" class={tooltipClass}>
              {fullLabel(ws, i)}
            </Tooltip.Content>
          </Tooltip.Portal>
        </Tooltip.Root>
      {/each}
    </div>

  {:else if mode === "dots"}
    <!--
      Dots mode (6-9 workspaces): compact dots, active dot is larger.
      Tooltip on hover shows workspace name.
    -->
    <div class="indicator" role="group" aria-label="Workspaces">
      {#each $workspaces as ws, i}
        <Tooltip.Root instant>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <button
                class="dot-btn"
                onclick={() => handleClick(ws.id)}
                aria-label={fullLabel(ws, i)}
                aria-pressed={ws.active}
                {...props}
              >
                <span class="dot" class:dot-active={ws.active}></span>
              </button>
            {/snippet}
          </Tooltip.Trigger>
          <Tooltip.Portal>
            <Tooltip.Content side="bottom" class={tooltipClass}>
              {fullLabel(ws, i)}
            </Tooltip.Content>
          </Tooltip.Portal>
        </Tooltip.Root>
      {/each}
    </div>

  {:else}
    <!--
      Text mode (10+ workspaces): compact "current / total".
    -->
    <div class="indicator" role="group" aria-label="Workspaces">
      <span class="ws-text" aria-label="Workspace {activeIndex >= 0 ? activeIndex + 1 : 1} of {$workspaces.length}">
        {activeIndex >= 0 ? activeIndex + 1 : 1} / {$workspaces.length}
      </span>
    </div>
  {/if}
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
    height: 24px;
    min-width: 32px;
    padding: 0 10px;
    border-radius: 12px;
    border: none;
    font-size: 0.6875rem;
    font-weight: 500;
    line-height: 1;
    cursor: pointer;
    white-space: nowrap;
    /* Crossfade on switch: 150ms */
    transition:
      background-color 150ms ease,
      color 150ms ease,
      transform 100ms ease;
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }

  .pill:hover {
    background: color-mix(in srgb, var(--foreground) 18%, transparent);
  }

  .pill:active {
    transform: scale(0.95);
    transition: transform 50ms ease;
  }

  .pill-active {
    background: var(--accent);
    color: var(--accent-foreground);
    /* Scale-in animation when workspace becomes active */
    animation: pill-activate 100ms ease forwards;
  }

  .pill-active:hover {
    background: color-mix(in srgb, var(--accent) 85%, var(--foreground) 15%);
  }

  @keyframes pill-activate {
    from { transform: scale(0.9); }
    to   { transform: scale(1); }
  }

  /* ── Dots ───────────────────────────────────────────────────────────── */

  .dot-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    background: transparent;
    cursor: pointer;
    border-radius: 50%;
    transition: transform 100ms ease;
  }

  .dot-btn:active {
    transform: scale(0.85);
  }

  .dot {
    display: block;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--foreground) 45%, transparent);
    transition:
      width 100ms ease,
      height 100ms ease,
      background-color 150ms ease;
  }

  .dot-btn:hover .dot {
    background: color-mix(in srgb, var(--foreground) 70%, transparent);
  }

  .dot-active {
    width: 7px;
    height: 7px;
    background: var(--accent);
    animation: dot-activate 100ms ease forwards;
  }

  .dot-btn:hover .dot-active {
    background: color-mix(in srgb, var(--accent) 85%, var(--foreground) 15%);
  }

  @keyframes dot-activate {
    from { transform: scale(0.7); }
    to   { transform: scale(1); }
  }

  /* ── Text ───────────────────────────────────────────────────────────── */

  .ws-text {
    font-size: 0.6875rem;
    font-weight: 500;
    color: var(--foreground);
    letter-spacing: 0.02em;
    pointer-events: none;
  }
</style>
