<script lang="ts">
  /// Quick Settings Panel (#52).

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import {
    Moon, Sun, Settings, Lock, Power, ChevronDown,
    LayoutTemplate, LayoutGrid, Bell, LogOut, RotateCcw,
  } from "lucide-svelte";

  let nightLightEnabled = $state(false);
  let brightness = $state(75);
  let layoutMode = $state<"float" | "tile">("float");
  let powerMenuOpen = $state(false);

  function handleKeydown(e: KeyboardEvent) {
    let current: string | null = null;
    activePopover.subscribe((v) => { current = v; })();
    if (current !== "quick-settings") return;
    if (e.key === "Escape") {
      e.preventDefault();
      if (powerMenuOpen) { powerMenuOpen = false; }
      else { closePopover(); }
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

{#if $activePopover === "quick-settings"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="qs-backdrop" onclick={() => { powerMenuOpen = false; closePopover(); }}></div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="qs-panel shell-popover" onclick={() => { powerMenuOpen = false; }}>

    <!-- ── Notifications ── -->
    <div class="qs-section">
      <span class="qs-heading">Notifications</span>
      <div class="qs-empty">
        <Bell size={28} strokeWidth={1} />
        <span>No notifications</span>
      </div>
    </div>

    <div class="qs-sep"></div>

    <!-- ── Quick Settings ── -->

    <!-- Night Light -->
    <div class="qs-row">
      <span class="qs-label">Night Light</span>
      <button
        class="qs-pill"
        class:active={nightLightEnabled}
        onclick={(e) => { e.stopPropagation(); nightLightEnabled = !nightLightEnabled; }}
        title={nightLightEnabled ? "Disable Night Light" : "Enable Night Light"}
      >
        {#if nightLightEnabled}
          <Moon size={14} strokeWidth={1.5} />
        {:else}
          <Sun size={14} strokeWidth={1.5} />
        {/if}
      </button>
    </div>

    <!-- Brightness -->
    <div class="qs-brightness-row">
      <Sun size={16} strokeWidth={1.5} class="qs-brightness-icon" />
      <div class="qs-slider" style="--value: {brightness}%">
        <div class="qs-slider-track"></div>
        <div class="qs-slider-fill"></div>
        <div class="qs-slider-thumb"></div>
        <input
          type="range"
          min="0"
          max="100"
          bind:value={brightness}
        />
      </div>
    </div>

    <!-- Layout -->
    <div class="qs-row">
      <span class="qs-label">Layout</span>
      <div class="qs-layout-pills">
        <button
          class="qs-pill"
          class:active={layoutMode === "float"}
          onclick={(e) => { e.stopPropagation(); layoutMode = "float"; }}
          title="Float"
        >
          <LayoutTemplate size={14} strokeWidth={1.5} />
        </button>
        <button
          class="qs-pill"
          class:active={layoutMode === "tile"}
          onclick={(e) => { e.stopPropagation(); layoutMode = "tile"; }}
          title="Tile"
        >
          <LayoutGrid size={14} strokeWidth={1.5} />
        </button>
      </div>
    </div>

    <div class="qs-sep"></div>

    <!-- ── User + Actions ── -->
    <div class="qs-user-row">
      <span class="qs-avatar">TK</span>
      <span class="qs-user-name">Tim Kicker</span>

      <button
        class="qs-icon-btn"
        onclick={(e) => { e.stopPropagation(); closePopover(); }}
        title="Settings"
      >
        <Settings size={16} strokeWidth={1.5} />
      </button>

      <div class="qs-power-dropdown">
        <button
          class="qs-icon-btn"
          onclick={(e) => { e.stopPropagation(); powerMenuOpen = !powerMenuOpen; }}
          title="Power"
        >
          <Power size={16} strokeWidth={1.5} />
          <ChevronDown size={10} strokeWidth={2} />
        </button>

        {#if powerMenuOpen}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <div class="qs-power-menu" onclick={(e) => e.stopPropagation()}>
            <button onclick={() => { powerMenuOpen = false; closePopover(); }}>
              <Lock size={14} strokeWidth={1.5} />
              <span>Lock</span>
            </button>
            <button onclick={() => { powerMenuOpen = false; closePopover(); }}>
              <LogOut size={14} strokeWidth={1.5} />
              <span>Log Out</span>
            </button>
            <div class="qs-power-sep"></div>
            <button onclick={() => { powerMenuOpen = false; closePopover(); }}>
              <RotateCcw size={14} strokeWidth={1.5} />
              <span>Restart</span>
            </button>
            <button onclick={() => { powerMenuOpen = false; closePopover(); }}>
              <Power size={14} strokeWidth={1.5} />
              <span>Shut Down</span>
            </button>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  .qs-backdrop {
    position: fixed;
    inset: 0;
    z-index: 90;
  }

  .qs-panel {
    position: fixed;
    top: 40px;
    right: 8px;
    z-index: 100;
    width: 340px;
    border-radius: 10px;
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    color: var(--color-fg-shell);
    overflow: visible;
    animation: qs-open 100ms ease-out both;
  }

  @keyframes qs-open {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  /* ── Section + Separator ── */

  .qs-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
  }

  .qs-heading {
    font-size: 0.6875rem;
    font-weight: 600;
    opacity: 0.5;
  }

  .qs-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 20px 0;
    opacity: 0.25;
    font-size: 0.6875rem;
  }

  .qs-sep {
    height: 1px;
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }

  /* ── Rows ── */

  .qs-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    min-height: 36px;
  }

  .qs-label {
    font-size: 0.75rem;
    color: var(--color-fg-shell);
  }

  /* ── Pill buttons (Night Light + Layout) ── */

  .qs-pill {
    width: 32px;
    height: 26px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-radius: 6px;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    cursor: pointer;
    padding: 0;
    transition: all 0.15s ease;
  }

  .qs-pill:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }

  .qs-pill.active {
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-fg-shell) 30%, transparent);
    color: var(--color-fg-shell);
  }

  .qs-layout-pills {
    display: flex;
    gap: 4px;
  }

  /* ── Custom Slider ── */

  .qs-brightness-row {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 12px;
  }

  :global(.qs-brightness-icon) {
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    flex-shrink: 0;
  }

  .qs-slider {
    position: relative;
    flex: 1;
    height: 20px;
    display: flex;
    align-items: center;
  }

  .qs-slider-track {
    position: absolute;
    left: 0;
    right: 0;
    height: 4px;
    background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    border-radius: 2px;
  }

  .qs-slider-fill {
    position: absolute;
    left: 0;
    width: var(--value);
    height: 4px;
    background: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    border-radius: 2px;
  }

  .qs-slider-thumb {
    position: absolute;
    left: var(--value);
    width: 14px;
    height: 14px;
    background: var(--color-fg-shell);
    border-radius: 50%;
    transform: translateX(-50%);
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
    pointer-events: none;
  }

  .qs-slider input[type="range"] {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
    margin: 0;
    -webkit-appearance: none;
  }

  /* ── Icon button ── */

  .qs-icon-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 2px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    cursor: pointer;
    transition: all 0.15s ease;
    padding: 0;
  }

  .qs-icon-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }

  /* ── User row ── */

  .qs-user-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
  }

  .qs-avatar {
    width: 32px;
    height: 32px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--color-fg-shell);
    flex-shrink: 0;
    user-select: none;
  }

  .qs-user-name {
    flex: 1;
    font-size: 0.8125rem;
    color: var(--color-fg-shell);
  }

  /* ── Power dropdown ── */

  .qs-power-dropdown {
    position: relative;
  }

  .qs-power-menu {
    position: absolute;
    bottom: 100%;
    right: 0;
    margin-bottom: 4px;
    min-width: 140px;
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    border-radius: 8px;
    padding: 4px;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
    z-index: 110;
  }

  .qs-power-menu button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--color-fg-shell);
    font-size: 0.75rem;
    cursor: pointer;
    text-align: left;
    transition: background-color 0.1s ease;
  }

  .qs-power-menu button:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }

  .qs-power-sep {
    height: 1px;
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    margin: 4px 0;
  }
</style>
