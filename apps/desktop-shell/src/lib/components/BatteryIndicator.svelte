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
  import { t } from "$lib/i18n/messages";
  import { durationText } from "$lib/duration";
  import { Applet, type AppletState } from "@arlen/ui-kit/components/topbar";
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

    // The power daemon's own reading, published on the bus as `power.state` and
    // forwarded here. Until now nothing listened to it and this indicator forked
    // UPower on a timer instead, which is the work the daemon exists to stop
    // doing nine times over.
    //
    // `observedAtMicros` is WHEN the reading was true. The bus retains the last
    // message of a state topic and hands it over on subscribe, so the first one
    // after a shell restart can be minutes old. Showing it is right - last known
    // beats blank - but it must not stop the refresh, so an old snapshot is
    // adopted AND asked to be re-read.
    const unlistenPower = listen<BatteryStatus & { observedAtMicros: number }>(
      "arlen://power-changed",
      (event) => {
        const p = event.payload;
        status = {
          percentage: p.percentage,
          charging: p.charging,
          time_remaining_minutes: p.time_remaining_minutes,
        };
        visible = true;
        const ageMs = Date.now() - p.observedAtMicros / 1000;
        if (ageMs > POLL_STALE_MS) {
          poll();
        } else {
          lastEventAt = Date.now();
        }
      },
    );
    const fallback = setInterval(() => {
      if (Date.now() - lastEventAt < POLL_STALE_MS) return;
      poll();
    }, 60_000);
    return () => {
      unlisten.then((fn) => fn());
      unlistenPower.then((fn) => fn());
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

  // One whole sentence per case rather than a level with clauses appended: the
  // charge clause is a phrase German puts in a different place ("noch 2 Std."),
  // so a translator needs the whole line, not the tail of one.
  const tooltip = $derived.by(() => {
    if (!status) return $t("sh.bat.tip.plain");
    const pct = status.percentage;
    const time = durationText($t, status.time_remaining_minutes);
    if (time) {
      return $t(status.charging ? "sh.bat.tip.untilFull" : "sh.bat.tip.remaining", {
        pct,
        time,
      });
    }
    if (status.charging) return $t("sh.bat.tip.charging", { pct });
    return $t("sh.bat.tip.level", { pct });
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
