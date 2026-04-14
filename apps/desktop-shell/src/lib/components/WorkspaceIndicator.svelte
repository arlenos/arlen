<script lang="ts">
  import { primaryWorkspaces, activateWorkspace, type WorkspaceInfo } from "$lib/stores/workspaces.js";
  import { windows, type WindowInfo } from "$lib/stores/windows.js";
  import { resolveAppIcon } from "$lib/stores/appIcons.js";
  import { invoke } from "@tauri-apps/api/core";
  import { AppWindow } from "lucide-svelte";

  const mode = $derived(
    $primaryWorkspaces.length <= 5 ? "pills" as const :
    $primaryWorkspaces.length <= 9 ? "dots" as const : "text" as const
  );

  const activeIndex = $derived($primaryWorkspaces.findIndex((w) => w.active));

  // Hover overlay state.
  let overlayVisible = $state(false);
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;

  function onEnter() {
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      overlayVisible = true;
      invoke("set_popover_input_region", { expanded: true }).catch(() => {});
    }, 600);
  }

  function onLeave() {
    if (hoverTimer) { clearTimeout(hoverTimer); hoverTimer = null; }
    // Short delay so the user can move to the overlay.
    hoverTimer = setTimeout(() => {
      overlayVisible = false;
      invoke("set_popover_input_region", { expanded: false }).catch(() => {});
    }, 300);
  }

  function onOverlayEnter() {
    if (hoverTimer) { clearTimeout(hoverTimer); hoverTimer = null; }
  }

  function onOverlayLeave() {
    overlayVisible = false;
    invoke("set_popover_input_region", { expanded: false }).catch(() => {});
  }

  function pillLabel(ws: WorkspaceInfo, i: number): string {
    const name = ws.name.trim();
    if (!name) return String(i + 1);
    return name.length > 12 ? name.slice(0, 12) + "\u2026" : name;
  }

  function fullLabel(ws: WorkspaceInfo, i: number): string {
    return ws.name.trim() || `Workspace ${i + 1}`;
  }

  function handleClick(id: string) {
    activateWorkspace(id);
  }

  function handleCardClick(id: string) {
    activateWorkspace(id);
    overlayVisible = false;
    invoke("set_popover_input_region", { expanded: false }).catch(() => {});
  }

  function getWindowsForWorkspace(wsId: string): WindowInfo[] {
    return $windows.filter((w) => w.workspace_ids.includes(wsId));
  }

  // Icon resolution cache.
  let iconUrls = $state<Record<string, string | null>>({});

  const allAppIds = $derived(
    [...new Set($windows.map((w) => w.app_id).filter(Boolean))]
  );

  $effect(() => {
    for (const appId of allAppIds) {
      if (!(appId in iconUrls)) {
        resolveAppIcon(appId).then((url) => {
          iconUrls = { ...iconUrls, [appId]: url };
        });
      }
    }
  });
</script>

{#if $primaryWorkspaces.length > 0}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="ws-root"
    onmouseenter={onEnter}
    onmouseleave={onLeave}
  >
    {#if mode === "pills"}
      <div class="indicator" role="group" aria-label="Workspaces">
        {#each $primaryWorkspaces as ws, i}
          <button
            class="pill"
            class:pill-active={ws.active}
            onclick={() => handleClick(ws.id)}
            aria-label={fullLabel(ws, i)}
            aria-pressed={ws.active}
          >
            {pillLabel(ws, i)}
          </button>
        {/each}
      </div>

    {:else if mode === "dots"}
      <div class="indicator" role="group" aria-label="Workspaces">
        {#each $primaryWorkspaces as ws, i}
          <button
            class="dot-btn"
            onclick={() => handleClick(ws.id)}
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
          {activeIndex >= 0 ? activeIndex + 1 : 1} / {$primaryWorkspaces.length}
        </span>
      </div>
    {/if}

    <!-- Hover overlay: workspace cards -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="overlay"
      class:overlay-visible={overlayVisible}
      onmouseenter={onOverlayEnter}
      onmouseleave={onOverlayLeave}
    >
      <div class="ws-cards">
        {#each $primaryWorkspaces as ws, i}
          {@const wsWindows = getWindowsForWorkspace(ws.id)}
          <button
            class="ws-card"
            class:ws-card-active={ws.active}
            onclick={() => handleCardClick(ws.id)}
            aria-label={fullLabel(ws, i)}
          >
            <span class="ws-card-name">
              {fullLabel(ws, i)}
            </span>
            {#if wsWindows.length > 0}
              <div class="ws-card-icons">
                {#each wsWindows.slice(0, 5) as win}
                  {#if iconUrls[win.app_id]}
                    <img
                      class="ws-app-icon"
                      src={iconUrls[win.app_id]}
                      alt={win.app_id}
                      width="24"
                      height="24"
                      style="border: 1px solid color-mix(in srgb, var(--color-fg-primary) 30%, transparent);"
                    />
                  {:else}
                    <AppWindow size={16} strokeWidth={1.5} class="ws-app-icon-fallback" />
                  {/if}
                {/each}
                {#if wsWindows.length > 5}
                  <span class="ws-overflow">+{wsWindows.length - 5}</span>
                {/if}
              </div>
            {:else}
              <span class="ws-empty">Empty</span>
            {/if}
          </button>
        {/each}
      </div>
    </div>
  </div>
{/if}

<style>
  .ws-root {
    position: relative;
    display: flex;
    align-items: center;
  }

  .indicator {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  /* ── Overlay ────────────────────────────────────────���───────────────── */

  .overlay {
    position: absolute;
    top: 100%;
    left: 50%;
    transform: translateX(-50%) translateY(4px);
    z-index: 50;
    padding: 8px;
    border-radius: var(--radius-lg);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    pointer-events: none;
    opacity: 0;
    transition:
      opacity 100ms ease,
      transform 100ms ease;
    transform-origin: top center;
  }

  .overlay-visible {
    opacity: 1;
    pointer-events: auto;
  }

  .ws-cards {
    display: flex;
    gap: 6px;
    overflow-x: auto;
  }

  .ws-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    min-width: 80px;
    max-width: 100px;
    padding: 8px 6px;
    border-radius: var(--radius-md);
    border: 1px solid transparent;
    background: transparent;
    cursor: pointer;
    transition:
      background-color 100ms ease,
      border-color 100ms ease;
    color: var(--color-fg-shell);
  }

  .ws-card:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }

  .ws-card-active {
    border-color: color-mix(in srgb, var(--color-accent) 30%, transparent);
    background: color-mix(in srgb, var(--color-accent) 8%, transparent);
  }

  .ws-card-name {
    font-size: 0.6875rem;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }

  .ws-card-icons {
    display: flex;
    align-items: center;
    gap: 3px;
    flex-wrap: wrap;
    justify-content: center;
  }

  .ws-app-icon {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
    object-fit: contain;
  }

  :global(.ws-app-icon-fallback) {
    width: 16px;
    height: 16px;
    opacity: 0.5;
  }

  .ws-overflow {
    font-size: 0.625rem;
    opacity: 0.5;
  }

  .ws-empty {
    font-size: 0.625rem;
    opacity: 0.35;
  }

  /* ── Pills ──────────────────────────────────────────────────────────── */

  .pill {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 24px;
    min-width: 32px;
    padding: 0 10px;
    border-radius: var(--radius-lg);
    border: none;
    font-size: 0.6875rem;
    font-weight: 500;
    line-height: 1;
    cursor: pointer;
    white-space: nowrap;
    transition:
      background-color 150ms ease,
      color 150ms ease,
      transform 100ms ease;
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
    animation: pill-activate 100ms ease forwards;
  }

  .pill-active:hover {
    background: color-mix(in srgb, var(--color-accent) 26%, transparent);
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
    border-radius: var(--radius-full);
    transition: transform 100ms ease;
  }

  .dot-btn:active {
    transform: scale(0.85);
  }

  .dot {
    display: block;
    width: 5px;
    height: 5px;
    border-radius: var(--radius-full);
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
    background: var(--color-accent);
    animation: dot-activate 100ms ease forwards;
  }

  .dot-btn:hover .dot-active {
    background: color-mix(in srgb, var(--color-accent) 85%, var(--color-fg-shell) 15%);
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
  }
</style>
