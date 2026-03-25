<script lang="ts">
  import { activeAppName } from "$lib/stores/windows.js";
  import { timeString } from "$lib/stores/status.js";
  import {
    networkState, volumeState, batteryState, notificationCount,
    type NetworkState, type VolumeState, type BatteryState,
  } from "$lib/stores/applets.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import {
    Wifi, WifiOff, EthernetPort,
    VolumeX, Volume1, Volume, Volume2,
    BatteryCharging, BatteryFull, BatteryMedium, BatteryLow, BatteryWarning,
    Bell,
  } from "lucide-svelte";

  // ── Dynamic icon selection ────────────────────────────────────────────────

  const NetworkIcon = $derived(
    $networkState === "ethernet"   ? EthernetPort :
    $networkState === "disconnected" ? WifiOff : Wifi
  );

  const networkLabel = $derived(
    $networkState === "ethernet"    ? "Ethernet" :
    $networkState === "disconnected" ? "No network" : "Wi-Fi"
  );

  const VolumeIcon = $derived(
    $volumeState === "muted" ? VolumeX :
    $volumeState === "low"   ? Volume1 :
    $volumeState === "high"  ? Volume2 : Volume
  );

  const volumeLabel = $derived(
    $volumeState === "muted" ? "Muted" :
    $volumeState === "low"   ? "Volume: low" :
    $volumeState === "high"  ? "Volume: high" : "Volume"
  );

  const BatteryIcon = $derived(
    $batteryState.charging       ? BatteryCharging :
    $batteryState.level >= 80    ? BatteryFull :
    $batteryState.level >= 40    ? BatteryMedium :
    $batteryState.level >= 15    ? BatteryLow : BatteryWarning
  );

  const batteryLabel = $derived(
    $batteryState.charging
      ? `Charging - ${$batteryState.level}%`
      : `Battery: ${$batteryState.level}%`
  );

  // ── Peek animation ────────────────────────────────────────────────────────

  type PeekApplet = "network" | "volume" | "battery" | "notifications";

  let peekingApplet = $state<PeekApplet | null>(null);
  let peekTimer: ReturnType<typeof setTimeout> | null = null;

  function triggerPeek(applet: PeekApplet) {
    if (peekTimer !== null) clearTimeout(peekTimer);
    peekingApplet = applet;
    // 150ms slide-in + 2000ms visible + 200ms slide-out = 2350ms
    peekTimer = setTimeout(() => {
      peekingApplet = null;
      peekTimer = null;
    }, 2350);
  }

  // Watch stores and trigger peek on state changes (not on initial render).
  let _networkInit = false;
  $effect(() => {
    const n = $networkState;
    if (!_networkInit) { _networkInit = true; return; }
    triggerPeek("network");
  });

  let _volumeInit = false;
  $effect(() => {
    const v = $volumeState;
    if (!_volumeInit) { _volumeInit = true; return; }
    triggerPeek("volume");
  });

  let _batteryInit = false;
  $effect(() => {
    const b = $batteryState;
    if (!_batteryInit) { _batteryInit = true; return; }
    triggerPeek("battery");
  });

  // Notification peek only when count increases (new notification).
  let _notifInit = false;
  let _prevNotifCount = 0;
  $effect(() => {
    const n = $notificationCount;
    if (!_notifInit) { _notifInit = true; _prevNotifCount = n; return; }
    if (n > _prevNotifCount) triggerPeek("notifications");
    _prevNotifCount = n;
  });

  // ── Tooltip content class (shared) ────────────────────────────────────────

  const tooltipClass = "border-border bg-popover text-popover-foreground rounded-md border px-2 py-0.5 text-xs shadow-md select-none";
</script>

<!--
  Top Bar: persistent 28px bar anchored to the top of the screen via wlr-layer-shell.

  Left:   Active app name (Phase 4B: Menubar when app registers menu via shell.menu API)
  Center: Empty - Focus Mode indicator added in Phase 4A
  Right:  Collapsed applets (expand on hover) + clock + quick settings
-->
<div class="topbar shell-surface" data-tauri-drag-region>

  <!-- Left: App name -->
  <div class="topbar-left" data-tauri-drag-region>
    <span class="app-name">
      {$activeAppName || "Lunaris"}
    </span>
  </div>

  <!-- Center: Focus Mode placeholder (Phase 4A) -->
  <div class="topbar-center" data-tauri-drag-region></div>

  <!-- Right: Collapsed applets + clock + settings -->
  <div class="topbar-right">

    <!-- Peek slot: absolutely positioned, shows only the changed icon during peek -->
    {#if peekingApplet !== null}
      <div class="peek-slot" aria-hidden="true">
        {#if peekingApplet === "network"}
          <NetworkIcon size={14} strokeWidth={1.5} class="text-foreground" />
        {:else if peekingApplet === "volume"}
          <VolumeIcon size={14} strokeWidth={1.5} class="text-foreground" />
        {:else if peekingApplet === "battery"}
          <BatteryIcon size={14} strokeWidth={1.5} class="text-foreground" />
        {:else if peekingApplet === "notifications"}
          <Bell size={14} strokeWidth={1.5} class="text-foreground" />
        {/if}
      </div>
    {/if}

    <!-- Collapsed applets: hidden by default, expand on hover -->
    <div class="applets-hidden">
      <Tooltip.Root>
        <Tooltip.Trigger>
          {#snippet child({ props })}
            <Button
              variant="ghost"
              size="icon-sm"
              class="applet-btn"
              aria-label={networkLabel}
              {...props}
            >
              <NetworkIcon size={14} strokeWidth={1.5} />
            </Button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content side="bottom" class={tooltipClass}>{networkLabel}</Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip.Root>

      <Tooltip.Root>
        <Tooltip.Trigger>
          {#snippet child({ props })}
            <Button
              variant="ghost"
              size="icon-sm"
              class="applet-btn"
              aria-label={volumeLabel}
              {...props}
            >
              <VolumeIcon size={14} strokeWidth={1.5} />
            </Button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content side="bottom" class={tooltipClass}>{volumeLabel}</Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip.Root>

      <Tooltip.Root>
        <Tooltip.Trigger>
          {#snippet child({ props })}
            <Button
              variant="ghost"
              size="icon-sm"
              class="applet-btn"
              aria-label={batteryLabel}
              {...props}
            >
              <BatteryIcon size={14} strokeWidth={1.5} />
            </Button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content side="bottom" class={tooltipClass}>{batteryLabel}</Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip.Root>

      <Tooltip.Root>
        <Tooltip.Trigger>
          {#snippet child({ props })}
            <Button
              variant="ghost"
              size="icon-sm"
              class="applet-btn relative"
              aria-label="Notifications"
              {...props}
            >
              <Bell size={14} strokeWidth={1.5} />
              {#if $notificationCount > 0}
                <span
                  class="pointer-events-none absolute -right-0.5 -top-0.5 flex size-3.5 items-center justify-center rounded-full bg-destructive text-[9px] font-bold leading-none text-white"
                  aria-label="{$notificationCount} unread"
                >
                  {$notificationCount > 9 ? "9+" : $notificationCount}
                </span>
              {/if}
            </Button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.Portal>
          <Tooltip.Content side="bottom" class={tooltipClass}>
            {$notificationCount > 0 ? `${$notificationCount} notification${$notificationCount > 1 ? "s" : ""}` : "Notifications"}
          </Tooltip.Content>
        </Tooltip.Portal>
      </Tooltip.Root>
    </div>

    <Separator orientation="vertical" class="mx-1 h-4 opacity-30" />

    <!-- Clock: always visible, click opens quick settings (popover in future) -->
    <Tooltip.Root>
      <Tooltip.Trigger>
        {#snippet child({ props })}
          <button
            class="clock transition-[background-color] duration-fast ease-default focus-visible:ring-ring/50 rounded focus-visible:outline-none focus-visible:ring-2"
            aria-label="Time"
            {...props}
          >
            <span class="time">{$timeString}</span>
          </button>
        {/snippet}
      </Tooltip.Trigger>
      <Tooltip.Portal>
        <Tooltip.Content side="bottom" class={tooltipClass}>Quick settings</Tooltip.Content>
      </Tooltip.Portal>
    </Tooltip.Root>

  </div>
</div>

<style>
  .topbar {
    display: flex;
    align-items: center;
    height: 36px;
    width: 100%;
    padding: 0 4px;
    position: relative;
    user-select: none;
    flex-shrink: 0;
    background: var(--background);
  }

  .topbar-left {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    min-width: 0;
  }

  .app-name {
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    padding-left: 8px;
  }

  .topbar-center {
    flex: 1;
    display: flex;
    justify-content: center;
    align-items: center;
  }

  .topbar-right {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1;
    justify-content: flex-end;
    padding-right: 4px;
    position: relative;
  }

  /* Collapsed applets: hidden by default, expand on hover of the right section. */
  .applets-hidden {
    display: flex;
    align-items: center;
    gap: 2px;
    max-width: 0;
    opacity: 0;
    overflow: hidden;
    /* Collapse: ease-exit (decelerate out) */
    transition:
      max-width var(--duration-medium) var(--easing-exit),
      opacity var(--duration-fast) var(--easing-default);
  }

  .topbar-right:hover .applets-hidden {
    max-width: 200px;
    opacity: 1;
    /* Expand: ease-enter (accelerate in) */
    transition:
      max-width var(--duration-medium) var(--easing-enter),
      opacity var(--duration-fast) var(--easing-default);
  }

  /* Applet button size: same as former .status-btn */
  .topbar-right :global(.applet-btn) {
    width: 26px;
    height: 22px;
    padding: 0;
    color: var(--foreground);
  }

  /* Clock button */
  .clock {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 0 6px;
    background: transparent;
    border: none;
    cursor: pointer;
    border-radius: 4px;
    height: 28px;
    justify-content: center;
  }

  .clock:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .time {
    font-size: 0.75rem;
    font-weight: 600;
    color: var(--foreground);
    line-height: 1.2;
  }

  .date {
    font-size: 0.625rem;
    color: var(--muted-foreground);
    line-height: 1.2;
  }

  /* Peek slot: absolutely positioned at the right edge (left of clock+settings area).
     Shows only the changed applet icon during state change animation.
     Width 26px + right offset 60px = just left of settings + clock cluster. */
  .peek-slot {
    position: absolute;
    right: 60px;
    top: 50%;
    transform: translateY(-50%);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 22px;
    pointer-events: none;
    animation: applet-peek 2350ms forwards;
  }

  @keyframes applet-peek {
    0% {
      transform: translateY(-50%) translateX(32px);
      opacity: 0;
      animation-timing-function: var(--easing-default);
    }
    6% {
      transform: translateY(-50%) translateX(0);
      opacity: 1;
      animation-timing-function: linear;
    }
    91% {
      transform: translateY(-50%) translateX(0);
      opacity: 1;
      animation-timing-function: cubic-bezier(0.4, 0.0, 1.0, 1);
    }
    100% {
      transform: translateY(-50%) translateX(32px);
      opacity: 0;
    }
  }
</style>
