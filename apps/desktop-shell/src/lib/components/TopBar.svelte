<script lang="ts">
  import GlobalMenuBar from "$lib/components/GlobalMenuBar.svelte";
  import ClockIndicator from "$lib/components/ClockIndicator.svelte";
  import NetworkIndicator from "$lib/components/NetworkIndicator.svelte";
  import AudioIndicator from "$lib/components/AudioIndicator.svelte";
  import BatteryIndicator from "$lib/components/BatteryIndicator.svelte";
  import BluetoothIndicator from "$lib/components/BluetoothIndicator.svelte";
  import BluetoothPopover from "$lib/components/BluetoothPopover.svelte";
  import TrayIndicator from "$lib/components/TrayIndicator.svelte";
  import TrayPopover from "$lib/components/TrayPopover.svelte";
  import PanelTrigger from "$lib/components/PanelTrigger.svelte";
  import QuickSettingsPanel from "$lib/components/QuickSettingsPanel.svelte";
  import NetworkPopover from "$lib/components/NetworkPopover.svelte";
  import AudioPopover from "$lib/components/AudioPopover.svelte";
  import BatteryPopover from "$lib/components/BatteryPopover.svelte";
  import WorkspaceIndicator from "$lib/components/WorkspaceIndicator.svelte";
  import SandboxedModuleIndicatorSlot from "$lib/components/SandboxedModuleIndicatorSlot.svelte";
  import LayoutIndicator from "$lib/components/LayoutIndicator.svelte";
  import LayoutPopover from "$lib/components/LayoutPopover.svelte";
  import { isFocused, focusState, deactivateFocus } from "$lib/stores/projects.js";
  import { X } from "lucide-svelte";
</script>

<!--
  z-index 95 keeps the bar (and its indicator buttons) above the
  popover backdrop (z-index 90) while still sitting below the
  popover panels (z-index 100). Without this, an open popover's
  backdrop would intercept hover events on the indicators, breaking
  the macOS-style hover-switch where moving the mouse from one
  applet to another should swap the visible popover without a click.
  Clicking the bar's background between buttons stays a no-op (the
  click does not reach the backdrop), matching menu-bar conventions.
-->
<div
  class="flex items-center justify-between h-9 w-full px-2 gap-4 relative select-none shrink-0 shell-surface"
  style="background: var(--background); z-index: 95;"
  data-tauri-drag-region
>
  <!-- LEFT: App menu + toolbar -->
  <div class="flex items-center gap-2 flex-1 min-w-0" data-tauri-drag-region>
    <GlobalMenuBar />
    <div class="slot-toolbar flex items-center gap-2"></div>
  </div>

  <!-- CENTER: Workspace indicator -->
  <div class="flex-none flex items-center justify-center" data-tauri-drag-region>
    <WorkspaceIndicator />
  </div>

  <!-- RIGHT: Tray + indicators + clock + panel -->
  <div class="flex items-center gap-2 flex-1 justify-end">
    <!-- SNI system tray -->
    <div class="slot-sni flex items-center gap-2">
      <TrayIndicator />
    </div>

    <!-- Focus mode project name -->
    <div class="slot-project flex items-center gap-1.5">
      {#if $isFocused}
        <div class="focus-indicator">
          {#if $focusState.accentColor}
            <!--
              Svelte `style:` directive rather than `style="..."` with
              {} interpolation. The Tailwind Vite plugin otherwise
              tries to CSS-parse the interpolation braces and trips
              up, surfacing as "Invalid declaration: <script lang=\"ts\">"
              on the script block a few lines above (known plugin
              bug — see CLAUDE.md "Tailwind v4 in Tauri/SvelteKit").
            -->
            <span class="focus-dot" style:background={$focusState.accentColor}></span>
          {/if}
          <span class="focus-name">{$focusState.projectName}</span>
        </div>
        <button class="focus-exit" onclick={() => deactivateFocus()} title="Exit Focus Mode">
          <X size={12} strokeWidth={2} />
        </button>
      {/if}
    </div>

    <!-- Region separator (hidden when slot-project is empty) -->
    <div class="region-sep"></div>

    <!-- Third-party module indicators -->
    <div class="slot-temp flex items-center gap-0.5">
      <SandboxedModuleIndicatorSlot />
    </div>

    <!-- System indicators -->
    <div class="flex items-center gap-0.5">
      <NetworkIndicator />
      <BluetoothIndicator />
      <AudioIndicator />
      <BatteryIndicator />
      <LayoutIndicator />
      <div class="topbar-sep"></div>
      <ClockIndicator />
      <PanelTrigger />
    </div>
  </div>
</div>

<!-- Popovers (rendered outside the bar, positioned fixed) -->
<LayoutPopover />
<NetworkPopover />
<AudioPopover />
<BatteryPopover />
<BluetoothPopover />
<TrayPopover />
<QuickSettingsPanel />

<style>
  /* Empty slots collapse */
  .slot-toolbar:empty,
  .slot-sni:empty,
  .slot-project:empty,
  .slot-temp:empty {
    display: none;
  }

  /* Region separator hides when adjacent slot-project is empty */
  .slot-project:empty + .region-sep {
    display: none;
  }

  .region-sep {
    width: 1px;
    height: 14px;
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    flex-shrink: 0;
  }

  .topbar-sep {
    width: 1px;
    height: 14px;
    background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    margin: 0 4px;
    flex-shrink: 0;
    align-self: center;
  }

  .focus-indicator {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 8px;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .focus-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    flex-shrink: 0;
  }
  .focus-name {
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--foreground);
    opacity: 0.85;
    white-space: nowrap;
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .focus-exit {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: var(--radius-sm);
    color: color-mix(in srgb, var(--foreground) 40%, transparent);
    cursor: pointer;
    transition: all 100ms ease;
  }
  .focus-exit:hover {
    background: color-mix(in srgb, var(--foreground) 15%, transparent);
    color: var(--foreground);
  }
</style>
