<script lang="ts">
  /// Battery indicator for the top bar.
  ///
  /// Polls UPower via Tauri every 30 seconds. Hidden when no battery
  /// is present (desktop PCs).

  import { invoke } from "@tauri-apps/api/core";
  import { togglePopover } from "$lib/stores/activePopover.js";
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

  // Initial poll + 30s interval.
  poll();
  let _interval: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (_interval) return;
    _interval = setInterval(poll, 30_000);
    return () => { if (_interval) { clearInterval(_interval); _interval = null; } };
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
    border-radius: 4px;
    cursor: pointer;
    color: var(--foreground);
    transition: background-color var(--duration-fast, 150ms) ease;
  }

  .battery-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
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
