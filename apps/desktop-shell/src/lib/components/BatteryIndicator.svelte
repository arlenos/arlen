<script lang="ts">
  /// Battery indicator for the top bar.
  ///
  /// Polls UPower via Tauri every 30 seconds. Hidden when no battery
  /// is present (desktop PCs).

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { togglePopover, hoverPopover } from "$lib/stores/activePopover.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import {
    BatteryCharging, BatteryFull, BatteryMedium, BatteryLow, BatteryWarning,
  } from "lucide-svelte";

  interface BatteryStatus {
    percentage: number;
    charging: boolean;
    time_remaining_minutes: number | null;
  }

  let status = $state<BatteryStatus | null>(null);
  let visible = $state(false);

  const tooltipClass =
    "rounded-md border px-2 py-0.5 text-xs shadow-md select-none"
    + " bg-[var(--color-bg-shell)] text-[var(--color-fg-shell)] border-[color-mix(in_srgb,var(--color-bg-shell)_60%,white_40%)]";

  async function poll() {
    try {
      const result = await invoke<BatteryStatus | null>("get_battery_status");
      status = result;
      visible = result !== null;
    } catch {
      visible = false;
    }
  }

  poll();

  // Event-freshness fallback: the UPower D-Bus monitor emits
  // `battery-changed` on every percentage tick. The timer below only
  // fires if no event has arrived within the freshness window.
  const POLL_STALE_MS = 180_000; // battery changes are slow; wider window
  let lastEventAt = Date.now();

  onMount(() => {
    const unlisten = listen("battery-changed", () => {
      lastEventAt = Date.now();
      poll();
    });
    const fallback = setInterval(() => {
      if (Date.now() - lastEventAt < POLL_STALE_MS) return;
      poll();
    }, 60_000);
    return () => {
      unlisten.then((fn) => fn());
      clearInterval(fallback);
    };
  });

  const Icon = $derived(
    !status ? BatteryFull :
    status.charging ? BatteryCharging :
    status.percentage >= 80 ? BatteryFull :
    status.percentage >= 40 ? BatteryMedium :
    status.percentage >= 15 ? BatteryLow :
    BatteryWarning
  );

  const colorClass = $derived(
    !status ? "" :
    status.percentage < 10 ? "battery-critical" :
    status.percentage < 20 ? "battery-warning" :
    ""
  );

  const showBadge = $derived(
    status !== null && (status.charging || status.percentage < 30)
  );

  const tooltipText = $derived(() => {
    if (!status) return "Battery";
    let text = `Battery: ${status.percentage}%`;
    if (status.time_remaining_minutes !== null && status.time_remaining_minutes > 0) {
      const h = Math.floor(status.time_remaining_minutes / 60);
      const m = status.time_remaining_minutes % 60;
      if (h > 0) {
        text += ` - ${h}h ${m}min ${status.charging ? "until full" : "remaining"}`;
      } else {
        text += ` - ${m}min ${status.charging ? "until full" : "remaining"}`;
      }
    } else if (status.charging) {
      text += " - Charging";
    }
    return text;
  });
</script>

{#if visible && status}
  <Tooltip.Root>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        <button
          class="battery-btn {colorClass}"
          aria-label={tooltipText()}
          {...props}
          onclick={() => togglePopover("battery")}
          onmouseenter={() => hoverPopover("battery")}
        >
          <Icon size={14} strokeWidth={1.5} />
          {#if showBadge && status}
            <span class="battery-badge">{status.percentage}</span>
          {/if}
        </button>
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Portal>
      <Tooltip.Content side="bottom" class={tooltipClass}>
        {tooltipText()}
      </Tooltip.Content>
    </Tooltip.Portal>
  </Tooltip.Root>
{/if}

<style>
  .battery-btn {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 24px;
    min-height: 24px;
    width: auto;
    height: 28px;
    padding: 0 4px;
    border: none;
    background: transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--foreground);
    transition:
      transform var(--duration-micro) var(--ease-out),
      background-color var(--duration-fast) var(--ease-out);
  }

  .battery-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .battery-btn:active {
    transform: scale(0.96);
  }

  .battery-badge {
    font-size: 0.5625rem;
    font-weight: 700;
    line-height: 1;
    color: var(--foreground);
    opacity: 0.7;
  }

  .battery-warning {
    color: var(--color-warning);
  }

  .battery-warning .battery-badge {
    color: var(--color-warning);
  }

  .battery-critical {
    color: var(--color-error);
  }

  .battery-critical .battery-badge {
    color: var(--color-error);
  }
</style>
