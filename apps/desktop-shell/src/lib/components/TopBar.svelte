<script lang="ts">
  import { onMount, onDestroy, setContext } from "svelte";
  import { writable } from "svelte/store";
  import type { Readable } from "svelte/store";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
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

  /// Per-output bar identity. The desktop-shell creates one
  /// WebviewWindow per monitor; each one mounts this component.
  /// `topbar_get_output` returns the registry entry the backend
  /// stamped on the window — including the `primary` flag.
  /// Secondary bars hide the system-indicators block (Audio,
  /// Network, Tray, QuickSettings) so we don't double-render the
  /// same global state on every screen.
  interface OutputInfo {
    gdkIndex: number;
    description: string;
    primary: boolean;
    /** Connector name (`DP-1`, …). May be `null` while the
     *  compositor's xdg-output name event is still pending. */
    connector: string | null;
  }

  // Default by window label, synchronously, so the first paint is
  // already correct. The only window labelled `main` is the one
  // bound to the primary monitor by `output_bars`; every dynamic
  // bar uses a `topbar-N` label and is by definition secondary.
  // This avoids a race where a fast-mounting secondary bar can see
  // `outputInfo === null` and fall through to a primary-rendering
  // default, briefly mounting tray + popovers + per-app D-Bus
  // subscribers it shouldn't have.
  const initialIsPrimary =
    typeof window !== "undefined" &&
    getCurrentWebviewWindow().label === "main";

  let outputInfo = $state<OutputInfo | null>(null);
  // `isPrimary` falls back to the label-derived value until the
  // registry replies. Once `outputInfo` is set we trust the
  // registry's `primary` flag (covers edge cases where `main` ends
  // up orphaned after a hot-plug).
  const isPrimary = $derived(
    outputInfo === null ? initialIsPrimary : outputInfo.primary,
  );

  // Per-output context published to children (WorkspaceIndicator,
  // GlobalMenuBar). The connector is `null` until the
  // `wayland_client` xdg-output table fills in; consumers fall
  // back to legacy global views for the brief startup window.
  const outputContext = writable<{
    connector: string | null;
    primary: boolean;
  }>({
    connector: null,
    primary: initialIsPrimary,
  });
  setContext<Readable<{ connector: string | null; primary: boolean }>>(
    "topbar-output",
    outputContext,
  );

  // Keep the context in lock-step with `outputInfo` so children
  // see updates as soon as the registry replies (or polls in).
  $effect(() => {
    outputContext.set({
      connector: outputInfo?.connector ?? null,
      primary: isPrimary,
    });
  });

  let unlistenOutputChanged: UnlistenFn | null = null;

  /// Re-fetch the registry entry. Called from mount, on each
  /// `lunaris://topbar-output-changed` event, AND on a 100 ms
  /// retry loop until the connector is resolved (xdg-output name
  /// arrival is asynchronous and can lag the WebView mount).
  /// `accept_null_connector` is true only for the primary bar —
  /// secondary bars MUST keep retrying until they have a
  /// connector, otherwise per-output filtering stays stuck on
  /// the global fallback.
  async function refetchOutputInfo(): Promise<OutputInfo | null> {
    try {
      const info = await invoke<OutputInfo | null>("topbar_get_output");
      if (info !== null) {
        outputInfo = info;
      }
      return info;
    } catch (err) {
      console.warn("topbar_get_output failed:", err);
      return null;
    }
  }

  onMount(async () => {
    // Subscribe to the backend's "registry changed" notifications
    // first so any change between mount and the initial fetch is
    // not missed.
    unlistenOutputChanged = await listen(
      "lunaris://topbar-output-changed",
      () => {
        refetchOutputInfo();
      },
    );

    // Retry until we have a registry entry. Then keep retrying
    // until the connector is non-null for secondary bars — the
    // primary bar is allowed to ship with connector=null because
    // its identity is already known via the `main` window label.
    for (let attempt = 0; attempt < 50; attempt++) {
      const info = await refetchOutputInfo();
      const acceptable =
        info !== null &&
        (info.primary || info.connector !== null);
      if (acceptable) return;
      await new Promise((r) => setTimeout(r, 100));
    }
    console.warn(
      "topbar: connector never resolved after 5s, per-output filters stay on label-derived fallback",
    );
  });

  onDestroy(() => {
    unlistenOutputChanged?.();
  });
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
    <!-- SNI system tray (primary bar only — single global tray
         instance avoids duplicating SNI clients per output) -->
    {#if isPrimary}
      <div class="slot-sni flex items-center gap-2">
        <TrayIndicator />
      </div>
    {/if}

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

    <!-- System indicators. Primary bar gets the full set; secondary
         bars only show clock so the user has time-of-day on every
         screen without duplicating Wayland subscribers + popovers. -->
    <div class="flex items-center gap-0.5">
      {#if isPrimary}
        <NetworkIndicator />
        <BluetoothIndicator />
        <AudioIndicator />
        <BatteryIndicator />
        <LayoutIndicator />
        <div class="topbar-sep"></div>
      {/if}
      <ClockIndicator />
      {#if isPrimary}
        <PanelTrigger />
      {/if}
    </div>
  </div>
</div>

<!-- Popovers (rendered outside the bar, positioned fixed). Only
     the primary bar mounts these — they'd otherwise pile up
     duplicate D-Bus / Wayland subscriptions on every output. -->
{#if isPrimary}
  <LayoutPopover />
  <NetworkPopover />
  <AudioPopover />
  <BatteryPopover />
  <BluetoothPopover />
  <TrayPopover />
  <QuickSettingsPanel />
{/if}

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
