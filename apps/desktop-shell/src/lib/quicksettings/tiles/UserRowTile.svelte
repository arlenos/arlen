<script lang="ts">
  /// Quick Settings footer row.
  ///
  /// Rendered as the last "tile" in the panel grid but visually a
  /// separate footer — no tile bg/border/radius. Holds account-level
  /// affordances that don't change daily (avatar + name, theme
  /// cycle, settings, power flyout).
  ///
  /// Theme cycle lives here instead of as its own tile because
  /// (a) it's an identity-level setting, (b) removing the lone Theme
  /// tile lets the four toggles above pair cleanly in two 2-up rows.
  import { Settings, Power, Lock, LogOut, RotateCcw, Sun, Moon } from "lucide-svelte";
  import { closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let powerOpen = $state(false);
  let isDark = $state(true);

  onMount(() => {
    refreshTheme();
    let stop: UnlistenFn | null = null;
    listen("lunaris://theme-changed", refreshTheme).then((u) => (stop = u));
    return () => stop?.();
  });

  async function refreshTheme() {
    try {
      const id = await invoke<string>("get_active_theme_id");
      isDark = id !== "light";
    } catch {}
  }

  async function cycleTheme() {
    try {
      await invoke("set_theme", { id: isDark ? "light" : "dark" });
      isDark = !isDark;
    } catch {}
  }

  function runQuickAction(id: string) {
    powerOpen = false;
    closePopover();
    invoke("quick_action_run", { id }).catch(() => {});
  }

  function openSettings() {
    closePopover();
    invoke("quick_action_run", { id: "qa.open_settings" }).catch(() => {});
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="user-row-footer">
  <div
    class="user-identity"
    role="button"
    tabindex="0"
    title="Account / power"
    onclick={(e) => {
      e.stopPropagation();
      powerOpen = !powerOpen;
    }}
  >
    <span class="user-avatar">TK</span>
    <span class="user-name">Tim Kicker</span>
  </div>

  <div class="user-actions">
    <button
      class="user-icon"
      title={isDark ? "Switch to light" : "Switch to dark"}
      onclick={(e) => {
        e.stopPropagation();
        cycleTheme();
      }}
    >
      {#if isDark}
        <Sun size={16} strokeWidth={1.5} />
      {:else}
        <Moon size={16} strokeWidth={1.5} />
      {/if}
    </button>
    <button
      class="user-icon"
      title="Settings"
      onclick={(e) => {
        e.stopPropagation();
        openSettings();
      }}
    >
      <Settings size={16} strokeWidth={1.5} />
    </button>
  </div>

  {#if powerOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="user-power" onclick={(e) => e.stopPropagation()}>
      <button onclick={() => runQuickAction("qa.lock_screen")}>
        <Lock size={14} strokeWidth={1.5} /><span>Lock</span>
      </button>
      <button onclick={() => runQuickAction("qa.logout")}>
        <LogOut size={14} strokeWidth={1.5} /><span>Log Out</span>
      </button>
      <div class="user-sep"></div>
      <button onclick={() => runQuickAction("qa.reboot")}>
        <RotateCcw size={14} strokeWidth={1.5} /><span>Restart</span>
      </button>
      <button class="danger" onclick={() => runQuickAction("qa.shutdown")}>
        <Power size={14} strokeWidth={1.5} /><span>Shut Down</span>
      </button>
    </div>
  {/if}
</div>

<style>
  /* Footer row, NOT a tile. The grid-cell wrapper still positions it
     full-row; this component only owns its inner layout/look. */
  .user-row-footer {
    grid-column: span 2;
    width: 100%;
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 4px 0 4px;
    margin-top: 4px;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }

  .user-identity {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    cursor: pointer;
    border-radius: var(--radius-input);
    padding: 6px 8px;
    transition: background-color 100ms ease;
  }
  .user-identity:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
  }
  .user-identity:focus-visible {
    outline: none;
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 35%, transparent);
  }
  .user-avatar {
    width: 28px;
    height: 28px;
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 0.6875rem;
    font-weight: 600;
    flex-shrink: 0;
    user-select: none;
  }
  .user-name {
    font-size: 0.8125rem;
  }

  .user-actions {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .user-icon {
    width: 30px;
    height: 30px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    border-radius: var(--radius-input);
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
    cursor: pointer;
    padding: 0;
    transition: all 100ms ease;
  }
  .user-icon:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }

  .user-power {
    position: absolute;
    bottom: 100%;
    left: 4px;
    margin-bottom: 6px;
    min-width: 160px;
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    border-radius: var(--radius-input);
    padding: 4px;
    box-shadow: var(--shadow-md);
    z-index: 110;
  }
  .user-power button {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    background: transparent;
    border: none;
    border-radius: var(--radius-input);
    color: var(--color-fg-shell);
    font-size: 0.75rem;
    cursor: pointer;
    text-align: left;
    transition: background-color 100ms ease;
  }
  .user-power button:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }
  .user-power button.danger {
    color: var(--color-error);
  }
  .user-power button.danger:hover {
    background: color-mix(in srgb, var(--color-error) 15%, transparent);
  }
  .user-sep {
    height: 1px;
    margin: 4px 0;
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }
</style>
