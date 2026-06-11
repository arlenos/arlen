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
  import NotificationsTrigger from "$lib/components/NotificationsTrigger.svelte";
  import NotificationsPopover from "$lib/components/NotificationsPopover.svelte";
  import QuickSettingsPanel from "$lib/components/QuickSettingsPanel.svelte";
  import NetworkPopover from "$lib/components/NetworkPopover.svelte";
  import AudioPopover from "$lib/components/AudioPopover.svelte";
  import BatteryPopover from "$lib/components/BatteryPopover.svelte";
  import WorkspaceIndicator from "$lib/components/WorkspaceIndicator.svelte";
  import SandboxedModuleIndicatorSlot from "$lib/components/SandboxedModuleIndicatorSlot.svelte";
  import LayoutIndicator from "$lib/components/LayoutIndicator.svelte";
  import LayoutPopover from "$lib/components/LayoutPopover.svelte";
  import ToolbarSlot from "$lib/components/ToolbarSlot.svelte";
  import CaffeineBadge from "$lib/components/topbar/badges/CaffeineBadge.svelte";
  import RecordingBadge from "$lib/components/topbar/badges/RecordingBadge.svelte";
  import NightLightBadge from "$lib/components/topbar/badges/NightLightBadge.svelte";
  import AirplaneBadge from "$lib/components/topbar/badges/AirplaneBadge.svelte";
  import { isFocused, focusState, deactivateFocus } from "$lib/stores/projects.js";
  import { closePopover } from "$lib/stores/activePopover.js";
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu/index.js";
  import { X, FolderSearch } from "lucide-svelte";

  /// Focus-mode chip click → opens the Waypointer with the
  /// `p:` prefix (project switcher). Same affordance as the QS
  /// Project tile so users have one consistent way to pick or
  /// switch projects.
  function openProjectSwitcher() {
    closePopover();
    invoke("set_query_and_show", { query: "p:", mode: "" }).catch(() => {});
  }

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
  /// `arlen://topbar-output-changed` event, AND on a 100 ms
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
      "arlen://topbar-output-changed",
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
<!--
  Shell-level applet sizing tokens are declared on `:root` in
  `app.css` so all Applet primitives in `sdk/ui-kit/topbar/`
  inherit them. The topbar imposes a consistent minimum hit-area
  regardless of the individual indicator's own opinions. Height is
  fixed because the topbar height is fixed (h-9 ≈ 36px); min-width
  keeps icon-only applets the same square 28×28 hit-target.
-->
<div
  class="flex items-center justify-between h-9 w-full px-2 gap-4 relative select-none shrink-0 shell-surface"
  style="background: var(--background); z-index: 95;"
  data-tauri-drag-region
>
  <!-- LEFT: App menu + toolbar -->
  <div class="flex items-center gap-2 flex-1 min-w-0" data-tauri-drag-region>
    <GlobalMenuBar />
    <div class="slot-toolbar flex items-center gap-2">
      <ToolbarSlot />
    </div>
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

    <!-- Focus-mode project chip. Left-click opens the project
         switcher (Waypointer p:), right-click opens a context
         menu with "Exit Focus Mode" + future capabilities
         (switch project, edit settings, pause focus, …). The
         old always-reserved X-button + spacer is gone — the
         exit affordance lives in the context menu where it
         doesn't take up bar space when focus is the user's
         steady state.
         The right-edge separator that used to sit here as
         `region-sep` was a duplicate of `topbar-sep` further
         down, so the slot now sits flush against the next
         group. -->
    <div class="slot-project flex items-center gap-1.5">
      {#if $isFocused}
        <ContextMenu.Root>
          <ContextMenu.Trigger>
            {#snippet child({ props })}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <div
                {...props}
                class="focus-indicator"
                role="button"
                tabindex="0"
                title={`Project: ${$focusState.projectName}. Right-click for options`}
                onclick={openProjectSwitcher}
              >
                {#if $focusState.accentColor}
                  <!--
                    Svelte `style:` directive rather than
                    `style="..."` with {} interpolation. The
                    Tailwind Vite plugin otherwise tries to
                    CSS-parse the interpolation braces and trips
                    up, surfacing as "Invalid declaration:
                    <script lang=\"ts\">" on the script block
                    a few lines above (known plugin bug — see
                    CLAUDE.md "Tailwind v4 in Tauri/SvelteKit").
                  -->
                  <span class="focus-dot" style:background={$focusState.accentColor}></span>
                {/if}
                <span class="focus-name">{$focusState.projectName}</span>
              </div>
            {/snippet}
          </ContextMenu.Trigger>
          <ContextMenu.Content class="shell-popover">
            <ContextMenu.Item onclick={openProjectSwitcher}>
              <FolderSearch size={14} strokeWidth={1.5} />
              <span>Switch project</span>
            </ContextMenu.Item>
            <ContextMenu.Separator />
            <ContextMenu.Item onclick={() => deactivateFocus()}>
              <X size={14} strokeWidth={1.5} />
              <span>Exit Focus Mode</span>
            </ContextMenu.Item>
          </ContextMenu.Content>
        </ContextMenu.Root>
      {/if}
    </div>

    <!-- Third-party module indicators -->
    <div class="slot-temp flex items-center gap-0.5">
      <SandboxedModuleIndicatorSlot />
    </div>

    <!-- Status badges. Each one self-mounts only when its underlying
         state is active. Order is fixed so the bar layout doesn't
         jitter as states flip on and off. The Focus indicator lives
         in `.slot-project` (above) instead of here — it already
         shows the project name + exit-button there, a duplicate
         status-badge would be redundant. -->
    {#if isPrimary}
      <div class="slot-status-badges flex items-center gap-0.5">
        <CaffeineBadge />
        <RecordingBadge />
        <NightLightBadge />
        <AirplaneBadge />
      </div>
    {/if}

    <!-- System indicators. Primary bar gets the full set; secondary
         bars only show clock so the user has time-of-day on every
         screen without duplicating Wayland subscribers + popovers.
         Order: live-info-cluster on the left (Notifications + Audio
         — both expand with transient context like unread counts and
         MPRIS metadata), stable status indicators (Net/BT/Battery/
         Layout) in the middle, fixed UI anchors (Clock + Settings)
         on the right after the trenner. The `Trenner + Calendar +
         Settings` triplet is fixed by design and must not be
         reordered. The first separator divides background-apps
         (SNI + slot-temp + status-badges) from system-icons. -->
    <div class="flex items-center gap-0.5">
      {#if isPrimary}
        <div class="topbar-sep"></div>
        <NotificationsTrigger />
        <AudioIndicator />
        <NetworkIndicator />
        <BluetoothIndicator />
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
  <NotificationsPopover />
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
    border-radius: var(--radius-chip);
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
  /* Focus-indicator chip is now interactive: left-click opens
     project-switcher, right-click opens context menu. Hover bg
     signals "this is interactive" without needing a separate
     button. */
  .focus-indicator {
    transition: background-color var(--duration-fast, 100ms) var(--ease-out, ease);
  }
  .focus-indicator:hover {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
  }
  .focus-indicator:focus-visible {
    outline: none;
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 35%, transparent);
  }
</style>
