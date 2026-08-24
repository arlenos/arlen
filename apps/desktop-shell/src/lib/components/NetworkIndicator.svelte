<script lang="ts">
  /// Network status indicator for the top bar.
  ///
  /// Wraps the shared `Applet` primitive — click/hover/tooltip/
  /// hit-target are shell-controlled. This component owns the
  /// nmcli polling, icon-by-state mapping, and the VPN corner
  /// badge.
  ///
  /// Polls nmcli via Tauri every 30s as a freshness fallback;
  /// the authoritative source is the `network-changed` event.

  import { invoke } from "@tauri-apps/api/core";
  import { togglePopover, hoverPopover, activePopover } from "$lib/stores/activePopover.js";
  import { Applet, AppletBadge } from "@arlen/ui-kit/components/topbar";
  import { Wifi, WifiOff, WifiZero, Cable, Shield, Plane } from "lucide-svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n/messages";
  import { onMount } from "svelte";

  interface NetworkStatus {
    connection_type: string;
    connected: boolean;
    name: string | null;
    signal_strength: number | null;
    vpn_active: boolean;
  }

  let status = $state<NetworkStatus | null>(null);
  let airplaneMode = $state(false);

  /// True when the network state could not be read at all.
  ///
  /// NOT THE SAME AS DISCONNECTED, and this indicator drew them identically
  /// until 20 August: a failed read became `status = null`, and `!status` chose
  /// the same `WifiOff` as `!status.connected`. On the shipped image, where
  /// `nmcli` is not installed, that made the bar claim the machine was offline
  /// while systemd-networkd had it online - a confident wrong statement in the
  /// most-glanced-at pixel on the screen. The popover beside this has always
  /// been careful about it (`sh.net.stateUnknown`, and no toggle when the radio
  /// state is unknown, because "a toggle renders a POSITION"); the icon was the
  /// part that guessed.
  let unknown = $state(false);

  async function poll() {
    const [air, net] = await Promise.all([
      // `undefined`, not `false`. A read that failed used to become "aeroplane
      // mode is off", which the icon then drew as an ordinary radio state - the
      // guess this component's own note says it stopped making, left in the one
      // branch that still made it.
      invoke<boolean>("get_airplane_mode")
        .then((a) => a as boolean | undefined)
        .catch(() => undefined),
      invoke<NetworkStatus>("get_network_status")
        .then((n) => n as NetworkStatus | null)
        .catch(() => undefined),
    ]);
    airplaneMode = air === true;
    unknown = net === undefined || air === undefined;
    status = air ? null : (net ?? null);
  }

  poll();

  const POLL_STALE_MS = 90_000;
  let lastEventAt = Date.now();

  onMount(() => {
    const unlisten = listen("network-changed", () => {
      lastEventAt = Date.now();
      poll();
    });
    const fallback = setInterval(() => {
      if (Date.now() - lastEventAt < POLL_STALE_MS) return;
      poll();
    }, 30_000);
    return () => {
      unlisten.then((fn) => fn());
      clearInterval(fallback);
    };
  });

  const Icon = $derived(
    airplaneMode
      ? Plane
      // Empty bars rather than the slash: the slash is a statement that there is
      // no connection, and this is the state where nobody knows.
      : unknown
        ? WifiZero
      : !status || !status.connected
        ? WifiOff
        : status.connection_type === "ethernet"
          ? Cable
          : Wifi,
  );

  /// Signal-strength → icon-opacity mapping. Weaker signal renders
  /// a subtler icon so the user's first-glance read of "how strong
  /// is the connection" matches their visual expectation. Bottoms
  /// at 40% so a 1-bar connection is still legible.
  const signalOpacity = $derived(
    status?.signal_strength != null
      ? Math.max(0.4, status.signal_strength / 100)
      : 1,
  );

  // Ethernet, WiFi and VPN stay as they are, and so does the percentage: the
  // same call the popover's detail line makes, where it is written down. What
  // is translated is the words - the two states, and the unnamed-network
  // fallback.
  const tooltip = $derived.by(() => {
    if (airplaneMode) return $t("sh.net.tip.airplane");
    if (!status || !status.connected) return $t("sh.net.disconnected");
    if (status.connection_type === "ethernet") {
      return `Ethernet: ${status.name ?? $t("sh.net.connected")}`;
    }
    let text = `WiFi: ${status.name ?? $t("sh.net.connected")}`;
    if (status.signal_strength != null) {
      text += ` (${status.signal_strength}%)`;
    }
    if (status.vpn_active) {
      text += " (VPN)";
    }
    return text;
  });

  const isOpen = $derived($activePopover === "network");
  const dimmed = $derived(!status?.connected && !airplaneMode);
</script>

<Applet
  appletId="network"
  {tooltip}
  popoverOpen={isOpen}
  {dimmed}
  state={airplaneMode ? "off" : status?.connected ? "on" : "off"}
  onclick={() => togglePopover("network")}
  onmouseenter={() => hoverPopover("network")}
>
  {#snippet icon()}
    <span style:opacity={signalOpacity}>
      <Icon size={14} strokeWidth={1.5} />
    </span>
  {/snippet}
  {#snippet badge()}
    {#if status?.vpn_active}
      <AppletBadge
        variant="icon"
        color="success"
        icon={vpnIcon}
      />
    {/if}
  {/snippet}
</Applet>

{#snippet vpnIcon()}
  <Shield size={9} strokeWidth={2.5} />
{/snippet}
