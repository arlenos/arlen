<script lang="ts">
  /// Quick Settings Panel with persistent config via shell.toml.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import {
    Moon, Sun, Plane, Settings, Lock, Power,
    LogOut, RotateCcw, Coffee, Circle, Sunset,
  } from "lucide-svelte";
  import NotificationPanel from "$lib/components/NotificationPanel.svelte";

  let isDark = $state(true);
  let caffeineActive = $state(false);
  let recordingActive = $state(false);

  async function toggleTheme() {
    const next = isDark ? "light" : "dark";
    try {
      await invoke("set_theme", { id: next });
      isDark = !isDark;
    } catch (e) {
      console.error("theme toggle failed:", e);
    }
  }

  async function toggleCaffeine() {
    try {
      caffeineActive = await invoke<boolean>("toggle_caffeine");
    } catch (e) {
      console.error("caffeine toggle failed:", e);
    }
  }

  async function toggleRecording() {
    try {
      recordingActive = await invoke<boolean>("toggle_recording");
    } catch (e) {
      console.error("recording toggle failed:", e);
    }
  }

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
    return () => {
      if (saveTimeout) clearTimeout(saveTimeout);
    };
  });

  $effect(() => {
    if ($activePopover === "quick-settings") {
      invoke<boolean>("get_airplane_mode").then((v) => { airplaneMode = v; }).catch(() => {});
      invoke<string>("get_active_theme_id").then((id) => { isDark = id !== "light"; }).catch(() => {});
      invoke<{ caffeineActive: boolean; recordingActive: boolean }>("get_toggle_status")
        .then((s) => { caffeineActive = s.caffeineActive; recordingActive = s.recordingActive; })
        .catch(() => {});
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
    // Don't go through `persistConfig` for night light: the
    // backend command persists shell.toml AND dispatches the
    // compositor request in one shot, which is what makes the
    // gamma engine warm the screen within the spec'd 200ms.
    invoke("night_light_set", {
      enabled: config.night_light.enabled,
      temperature: config.night_light.temperature,
    }).catch((err) => {
      console.warn("night_light_set failed:", err);
    });
  }

  function setBrightness(value: number) {
    config.display.brightness = value / 100;
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

    <!-- 1. User Row -->
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
            <Lock size={14} strokeWidth={1.5} /><span>Lock</span>
          </button>
          <button onclick={() => { powerMenuOpen = false; closePopover(); }}>
            <LogOut size={14} strokeWidth={1.5} /><span>Log Out</span>
          </button>
          <div class="qs-sep"></div>
          <button onclick={() => { powerMenuOpen = false; closePopover(); }}>
            <RotateCcw size={14} strokeWidth={1.5} /><span>Restart</span>
          </button>
          <button class="qs-power-danger" onclick={() => { powerMenuOpen = false; closePopover(); }}>
            <Power size={14} strokeWidth={1.5} /><span>Shut Down</span>
          </button>
        </div>
      {/if}
    </div>

    <div class="qs-sep"></div>

    <!-- 2. Quick Toggles -->
    <div class="qs-toggles">
      <button
        class="qs-toggle-btn"
        class:active={!isDark}
        onclick={(e) => { e.stopPropagation(); toggleTheme(); }}
        title={isDark ? "Switch to Light Mode" : "Switch to Dark Mode"}
      >
        {#if isDark}
          <Sun size={16} strokeWidth={1.5} />
        {:else}
          <Moon size={16} strokeWidth={1.5} />
        {/if}
      </button>

      <button
        class="qs-toggle-btn"
        class:active={config.night_light.enabled}
        onclick={(e) => { e.stopPropagation(); toggleNightLight(); }}
        title={config.night_light.enabled ? "Disable Night Light" : "Enable Night Light"}
      >
        <Sunset size={16} strokeWidth={1.5} />
      </button>

      <button
        class="qs-toggle-btn"
        class:active={airplaneMode}
        onclick={(e) => { e.stopPropagation(); toggleAirplaneMode(); }}
        title={airplaneMode ? "Disable Airplane Mode" : "Enable Airplane Mode"}
      >
        <Plane size={16} strokeWidth={1.5} />
      </button>

      <button
        class="qs-toggle-btn"
        class:active={caffeineActive}
        onclick={(e) => { e.stopPropagation(); toggleCaffeine(); }}
        title={caffeineActive ? "Disable Caffeine" : "Enable Caffeine"}
      >
        <Coffee size={16} strokeWidth={1.5} />
      </button>

      <button
        class="qs-toggle-btn"
        class:active={recordingActive}
        class:recording={recordingActive}
        onclick={(e) => { e.stopPropagation(); toggleRecording(); }}
        title={recordingActive ? "Stop Recording" : "Start Recording"}
      >
        <Circle size={16} strokeWidth={1.5} />
      </button>
    </div>

    <!-- 3. Brightness -->
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

    <!-- 4. Notifications -->
    <div class="qs-section">
      <NotificationPanel />
    </div>
  </div>
{/if}

<style>
  .qs-backdrop { position: fixed; inset: 0; z-index: 90; }

  .qs-panel {
    position: fixed; top: 40px; right: 8px; z-index: 100; width: 340px;
    border-radius: var(--radius-lg); background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg); color: var(--color-fg-shell);
    overflow: visible;
    animation: lunaris-popover-in var(--duration-medium) var(--ease-out) both;
    transform-origin: top center;
  }
  /* Entry keyframes defined in sdk/ui-kit/src/lib/motion.css. */

  .qs-sep { height: 1px; background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }

  /* User row */
  .qs-user-row { display: flex; align-items: center; gap: 8px; padding: 10px 12px; position: relative; }
  .qs-user-trigger { display: flex; align-items: center; gap: 10px; flex: 1; cursor: pointer; border-radius: var(--radius-md); padding: 2px; margin: -2px; transition: background-color 100ms ease; }
  .qs-user-trigger:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .qs-avatar { width: 32px; height: 32px; border-radius: var(--radius-lg); background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent); display: flex; align-items: center; justify-content: center; font-size: 0.6875rem; font-weight: 600; flex-shrink: 0; user-select: none; }
  .qs-user-name { flex: 1; font-size: 0.8125rem; }
  .qs-icon-btn { width: 28px; height: 28px; display: flex; align-items: center; justify-content: center; background: transparent; border: none; border-radius: var(--radius-md); color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent); cursor: pointer; transition: all 100ms ease; padding: 0; }
  .qs-icon-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }

  /* Power menu */
  .qs-power-menu { position: absolute; top: 100%; left: 12px; margin-top: 4px; min-width: 160px; background: var(--color-bg-shell); border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent); border-radius: var(--radius-md); padding: 4px; box-shadow: var(--shadow-md); z-index: 110; }
  .qs-power-menu button { width: 100%; display: flex; align-items: center; gap: 8px; padding: 8px 10px; background: transparent; border: none; border-radius: var(--radius-md); color: var(--color-fg-shell); font-size: 0.75rem; cursor: pointer; text-align: left; transition: background-color 100ms ease; }
  .qs-power-menu button:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .qs-power-danger { color: var(--color-error) !important; }
  .qs-power-danger:hover { background: color-mix(in srgb, var(--color-error) 15%, transparent) !important; }

  /* Toggle buttons - same pattern as bt-scan-btn, net-item etc. */
  .qs-toggles { display: flex; gap: 6px; padding: 10px 12px; }
  .qs-toggle-btn {
    width: 40px; height: 36px; display: flex; align-items: center; justify-content: center;
    background: transparent; border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-radius: var(--radius-md); color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    cursor: pointer; padding: 0; transition: all 100ms ease;
  }
  .qs-toggle-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }
  .qs-toggle-btn.active { background: color-mix(in srgb, var(--color-accent) 15%, transparent); border-color: color-mix(in srgb, var(--color-accent) 30%, transparent); color: var(--color-fg-shell); }
  .qs-toggle-btn.recording { color: var(--color-error); border-color: color-mix(in srgb, var(--color-error) 40%, transparent); animation: qs-pulse 1.5s ease-in-out infinite; }
  @keyframes qs-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.5; } }

  /* Brightness slider - same pattern as pop-slider in AudioPopover */
  .qs-brightness-row { display: flex; align-items: center; gap: 12px; padding: 8px 12px; }
  :global(.qs-brightness-icon) { color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent); flex-shrink: 0; }
  .qs-slider { position: relative; flex: 1; height: 20px; display: flex; align-items: center; }
  .qs-slider-track { position: absolute; left: 0; right: 0; height: 4px; background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent); border-radius: var(--radius-sm); }
  .qs-slider-fill { position: absolute; left: 0; width: var(--value); height: 4px; background: var(--color-accent); border-radius: var(--radius-sm); }
  .qs-slider-thumb { position: absolute; left: var(--value); width: 14px; height: 14px; background: var(--color-fg-shell); border-radius: var(--radius-md); transform: translateX(-50%); box-shadow: var(--shadow-sm); pointer-events: none; }
  .qs-slider input[type="range"] { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; cursor: pointer; margin: 0; appearance: none; -webkit-appearance: none; }


  /* Notifications */
  .qs-section { display: flex; flex-direction: column; gap: 8px; padding: 12px; }
</style>
