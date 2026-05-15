<script lang="ts">
  /// Battery indicator for the top bar.
  ///
  /// Wraps the shared `Applet` primitive. Polls UPower via Tauri
  /// (event-driven with a freshness fallback). Hidden when no
  /// battery is present (desktop PCs).
  ///
  /// The percentage shows as an inline label (right of the icon)
  /// only when the level is low or when charging — the most
  /// information-dense states. At regular levels (>30%, not
  /// charging) the icon alone communicates "fine, full enough".
  ///
  /// Semantic state: `warn` for <20%, `error` for <10%, `on` for
  /// charging, `off` otherwise. The Applet primitive maps these
  /// to icon colours via the `state` token.

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { togglePopover, hoverPopover, activePopover } from "$lib/stores/activePopover.js";
  import { Applet, type AppletState } from "@lunaris/ui-kit/components/topbar";
  import {
    BatteryCharging,
    BatteryFull,
    BatteryMedium,
    BatteryLow,
    BatteryWarning,
  } from "lucide-svelte";

  interface BatteryStatus {
    percentage: number;
    charging: boolean;
    time_remaining_minutes: number | null;
  }

  let status = $state<BatteryStatus | null>(null);
  let visible = $state(false);

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

  const POLL_STALE_MS = 180_000;
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
    !status
      ? BatteryFull
      : status.charging
        ? BatteryCharging
        : status.percentage >= 80
          ? BatteryFull
          : status.percentage >= 40
            ? BatteryMedium
            : status.percentage >= 15
              ? BatteryLow
              : BatteryWarning,
  );

  const showLabel = $derived(
    status !== null && (status.charging || status.percentage < 30),
  );

  const appletStateValue: AppletState | undefined = $derived(
    !status
      ? undefined
      : status.percentage < 10
        ? "error"
        : status.percentage < 20
          ? "warn"
          : status.charging
            ? "on"
            : undefined,
  );

  const tooltip = $derived.by(() => {
    if (!status) return "Battery";
    let text = `Battery: ${status.percentage}%`;
    if (
      status.time_remaining_minutes !== null &&
      status.time_remaining_minutes > 0
    ) {
      const h = Math.floor(status.time_remaining_minutes / 60);
      const m = status.time_remaining_minutes % 60;
      if (h > 0) {
        text += ` — ${h}h ${m}min ${status.charging ? "until full" : "remaining"}`;
      } else {
        text += ` — ${m}min ${status.charging ? "until full" : "remaining"}`;
      }
    } else if (status.charging) {
      text += " — Charging";
    }
    return text;
  });

  const isOpen = $derived($activePopover === "battery");
</script>

{#if visible && status}
  <Applet
    appletId="battery"
    {tooltip}
    popoverOpen={isOpen}
    state={appletStateValue}
    label={showLabel ? `${status.percentage}` : undefined}
    onclick={() => togglePopover("battery")}
    onmouseenter={() => hoverPopover("battery")}
  >
    {#snippet icon()}
      <Icon size={14} strokeWidth={1.5} />
    {/snippet}
  </Applet>
{/if}
