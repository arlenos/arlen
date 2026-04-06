<script lang="ts">
  /// Quick Settings Panel with persistent config via shell.toml.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import {
    Moon, Sun, Plane, Settings, Lock, Power,
    LayoutTemplate, LayoutGrid, Bell, LogOut, RotateCcw,
  } from "lucide-svelte";

  interface ShellConfig {
    night_light: { enabled: boolean; temperature: number };
    display: { brightness: number };
    layout: { mode: string };
  }

  let config = $state<ShellConfig>({
    night_light: { enabled: false, temperature: 3400 },
    display: { brightness: 1.0 },
    layout: { mode: "float" },
  });

  let airplaneMode = $state(false);
  let powerMenuOpen = $state(false);
  let saveTimeout: ReturnType<typeof setTimeout> | null = null;

  const brightnessPercent = $derived(Math.round(config.display.brightness * 100));

  onMount(() => {
    invoke<ShellConfig>("get_shell_config")
      .then((c) => { config = c; })
      .catch(() => {});
  });

  $effect(() => {
    if ($activePopover === "quick-settings") {
      invoke<boolean>("get_airplane_mode").then((v) => { airplaneMode = v; }).catch(() => {});
    }
  });

  function persistConfig() {
    if (saveTimeout) clearTimeout(saveTimeout);
    saveTimeout = setTimeout(() => {
      invoke("save_shell_config", { config }).catch(() => {});
    }, 500);
  }

  function toggleNightLight() {
    config.night_light.enabled = !config.night_light.enabled;
    persistConfig();
  }

  function setBrightness(value: number) {
    config.display.brightness = value / 100;
    persistConfig();
  }

  function setLayout(mode: string) {
    config.layout.mode = mode;
    persistConfig();
  }

  async function toggleAirplaneMode() {
    try {
      await invoke("set_airplane_mode", { enabled: !airplaneMode });
      airplaneMode = !airplaneMode;
    } catch {}
  }

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

    <!-- User row with power menu -->
    <div class="qs-user-row">
      <div class="qs-user-trigger" role="button" tabindex="-1"
        onclick={(e) => { e.stopPropagation(); powerMenuOpen = !powerMenuOpen; }}
      >
        <span class="qs-avatar">TK</span>
        <span class="qs-user-name">Tim Kicker</span>
      </div>

      <button
        class="qs-icon-btn"
        onclick={(e) => { e.stopPropagation(); closePopover(); }}
        title="Settings"
      >
        <Settings size={16} strokeWidth={1.5} />
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
          <button class="qs-power-danger" onclick={() => { powerMenuOpen = false; closePopover(); }}>
            <Power size={14} strokeWidth={1.5} />
            <span>Shut Down</span>
          </button>
        </div>
      {/if}
    </div>

    <div class="qs-sep"></div>

    <!-- Quick toggles (icon-only) -->
    <div class="qs-toggles">
      <button
        class="qs-toggle-btn"
        class:active={config.night_light.enabled}
        onclick={(e) => { e.stopPropagation(); toggleNightLight(); }}
        title={config.night_light.enabled ? "Disable Night Light" : "Enable Night Light"}
      >
        {#if config.night_light.enabled}
          <Moon size={16} strokeWidth={1.5} />
        {:else}
          <Sun size={16} strokeWidth={1.5} />
        {/if}
      </button>

      <button
        class="qs-toggle-btn"
        class:active={airplaneMode}
        onclick={(e) => { e.stopPropagation(); toggleAirplaneMode(); }}
        title={airplaneMode ? "Disable Airplane Mode" : "Enable Airplane Mode"}
      >
        <Plane size={16} strokeWidth={1.5} />
      </button>
    </div>

    <!-- Brightness -->
    <div class="qs-brightness-row">
      <Sun size={16} strokeWidth={1.5} class="qs-brightness-icon" />
      <div class="qs-slider" style="--value: {brightnessPercent}%">
        <div class="qs-slider-track"></div>
        <div class="qs-slider-fill"></div>
        <div class="qs-slider-thumb"></div>
        <input
          type="range"
          min="0"
          max="100"
          value={brightnessPercent}
          oninput={(e) => setBrightness(parseInt(e.currentTarget.value))}
        />
      </div>
    </div>

    <!-- Layout -->
    <div class="qs-row">
      <span class="qs-label">Layout</span>
      <div class="qs-layout-pills">
        <button
          class="qs-pill"
          class:active={config.layout.mode === "float"}
          onclick={(e) => { e.stopPropagation(); setLayout("float"); }}
          title="Float"
        >
          <LayoutTemplate size={14} strokeWidth={1.5} />
        </button>
        <button
          class="qs-pill"
          class:active={config.layout.mode === "tile"}
          onclick={(e) => { e.stopPropagation(); setLayout("tile"); }}
          title="Tile"
        >
          <LayoutGrid size={14} strokeWidth={1.5} />
        </button>
      </div>
    </div>

    <div class="qs-sep"></div>

    <!-- Notifications -->
    <div class="qs-section">
      <span class="qs-heading">Notifications</span>
      <div class="qs-empty">
        <Bell size={28} strokeWidth={1} />
        <span>No notifications</span>
      </div>
    </div>
  </div>
{/if}

<style>
  .qs-backdrop { position: fixed; inset: 0; z-index: 90; }

  .qs-panel {
    position: fixed; top: 40px; right: 8px; z-index: 100; width: 340px;
    border-radius: var(--radius-lg, 12px); background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg); color: var(--color-fg-shell);
    overflow: visible; animation: qs-open 100ms ease-out both;
  }
  @keyframes qs-open { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }

  /* User row */
  .qs-user-row { display: flex; align-items: center; gap: 8px; padding: 10px 12px; position: relative; }
  .qs-user-trigger { display: flex; align-items: center; gap: 10px; flex: 1; cursor: pointer; border-radius: var(--radius-md, 8px); padding: 2px; margin: -2px; transition: background-color var(--duration-fast) ease; }
  .qs-user-trigger:hover { background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent); }
  .qs-avatar { width: 32px; height: 32px; border-radius: var(--radius-full, 9999px); background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent); display: flex; align-items: center; justify-content: center; font-size: 0.6875rem; font-weight: 600; color: var(--color-fg-shell); flex-shrink: 0; user-select: none; }
  .qs-user-name { flex: 1; font-size: 0.8125rem; color: var(--color-fg-shell); }
  .qs-icon-btn { width: 28px; height: 28px; display: flex; align-items: center; justify-content: center; background: transparent; border: 1px solid transparent; border-radius: var(--radius-md, 8px); color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent); cursor: pointer; transition: all var(--duration-fast) ease; padding: 0; }
  .qs-icon-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }

  /* Power menu */
  .qs-power-menu { position: absolute; top: 100%; left: 12px; margin-top: 4px; min-width: 160px; background: var(--color-bg-shell); border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent); border-radius: var(--radius-md, 8px); padding: 4px; box-shadow: var(--shadow-md); z-index: 110; }
  .qs-power-menu button { width: 100%; display: flex; align-items: center; gap: 8px; padding: 8px 10px; background: transparent; border: none; border-radius: var(--radius-md, 8px); color: var(--color-fg-shell); font-size: 0.75rem; cursor: pointer; text-align: left; transition: background-color var(--duration-fast) ease; }
  .qs-power-menu button:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .qs-power-danger { color: var(--color-error) !important; }
  .qs-power-danger:hover { background: color-mix(in srgb, var(--color-error) 15%, transparent) !important; }
  .qs-power-sep { height: 1px; background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); margin: 4px 0; }

  .qs-sep { height: 1px; background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }

  /* Quick toggles (icon-only row) */
  .qs-toggles { display: flex; gap: 6px; padding: 10px 12px; }
  .qs-toggle-btn {
    width: 40px; height: 36px; display: flex; align-items: center; justify-content: center;
    background: transparent; border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-radius: var(--radius-md, 8px); color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    cursor: pointer; padding: 0; transition: all var(--duration-fast) ease;
  }
  .qs-toggle-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }
  .qs-toggle-btn.active { background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent); border-color: color-mix(in srgb, var(--color-fg-shell) 30%, transparent); color: var(--color-fg-shell); }

  /* Brightness */
  .qs-brightness-row { display: flex; align-items: center; gap: 12px; padding: 8px 12px; }
  :global(.qs-brightness-icon) { color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent); flex-shrink: 0; }
  .qs-slider { position: relative; flex: 1; height: 20px; display: flex; align-items: center; }
  .qs-slider-track { position: absolute; left: 0; right: 0; height: 4px; background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent); border-radius: 2px; }
  .qs-slider-fill { position: absolute; left: 0; width: var(--value); height: 4px; background: color-mix(in srgb, var(--color-fg-shell) 60%, transparent); border-radius: 2px; }
  .qs-slider-thumb { position: absolute; left: var(--value); width: 14px; height: 14px; background: var(--color-fg-shell); border-radius: var(--radius-full, 9999px); transform: translateX(-50%); box-shadow: var(--shadow-sm); pointer-events: none; }
  .qs-slider input[type="range"] { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; cursor: pointer; margin: 0; appearance: none; -webkit-appearance: none; }

  /* Layout row */
  .qs-row { display: flex; align-items: center; justify-content: space-between; padding: 8px 12px; min-height: 36px; }
  .qs-label { font-size: 0.75rem; color: var(--color-fg-shell); }
  .qs-pill {
    width: 32px; height: 26px; display: flex; align-items: center; justify-content: center;
    background: transparent; border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-radius: var(--radius-md, 8px); color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    cursor: pointer; padding: 0; transition: all var(--duration-fast) ease;
  }
  .qs-pill:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }
  .qs-pill.active { background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent); border-color: color-mix(in srgb, var(--color-fg-shell) 30%, transparent); color: var(--color-fg-shell); }
  .qs-layout-pills { display: flex; gap: 4px; }

  /* Notifications */
  .qs-section { display: flex; flex-direction: column; gap: 8px; padding: 12px; }
  .qs-heading { font-size: 0.6875rem; font-weight: 600; opacity: 0.5; }
  .qs-empty { display: flex; flex-direction: column; align-items: center; gap: 6px; padding: 20px 0; opacity: 0.25; font-size: 0.6875rem; }
</style>
