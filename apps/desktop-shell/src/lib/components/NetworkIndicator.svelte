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
  import { Wifi, WifiOff, Cable, Shield, Plane } from "lucide-svelte";
  import { listen } from "@tauri-apps/api/event";
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

  async function poll() {
    const [air, net] = await Promise.all([
      invoke<boolean>("get_airplane_mode").catch(() => false),
      invoke<NetworkStatus>("get_network_status").catch(() => null),
    ]);
    airplaneMode = air;
    status = air ? null : net;
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

  const tooltip = $derived.by(() => {
    if (airplaneMode) return "Airplane Mode";
    if (!status || !status.connected) return "Disconnected";
    if (status.connection_type === "ethernet") {
      return `Ethernet: ${status.name ?? "Connected"}`;
    }
    let text = `WiFi: ${status.name ?? "Connected"}`;
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
