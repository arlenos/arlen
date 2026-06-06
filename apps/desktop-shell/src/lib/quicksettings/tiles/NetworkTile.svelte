<script lang="ts">
  /// QS tile: WiFi / Cable / Airplane status.
  ///
  /// Single-click toggles WiFi on/off (the most likely action for
  /// the user). Right-click opens the existing NetworkPopover for
  /// network picking, VPN, connection details. Slider-style sub-
  /// states (signal strength, security) live in the popover.
  import { BaseTile } from "@lunaris/ui-kit/components/quicksettings";
  import { Wifi, WifiOff, Cable, Plane } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { openPopover } from "$lib/stores/activePopover.js";

  interface NetworkStatus {
    connection_type: string;
    connected: boolean;
    name: string | null;
    signal_strength: number | null;
    vpn_active: boolean;
  }

  let status = $state<NetworkStatus | null>(null);
  let airplane = $state(false);
  let wifiEnabled = $state(true);

  onMount(() => {
    refresh();
    let stopNet: UnlistenFn | null = null;
    let stopAir: UnlistenFn | null = null;
    listen("lunaris://network-changed", refresh).then((u) => (stopNet = u));
    listen("lunaris://airplane-changed", refresh).then((u) => (stopAir = u));
    const interval = setInterval(refresh, 30_000);
    return () => {
      clearInterval(interval);
      stopNet?.();
      stopAir?.();
    };
  });

  async function refresh() {
    try {
      airplane = await invoke<boolean>("get_airplane_mode");
      wifiEnabled = await invoke<boolean>("get_wifi_enabled");
      status = await invoke<NetworkStatus>("get_network_status");
    } catch {}
  }

  /// Single-click: toggle WiFi specifically. Ethernet stays unaffected
  /// because the user usually wants to flip wireless in isolation.
  /// Airplane mode is a separate tile + topbar badge.
  async function handleClick() {
    if (airplane) return;
    try {
      await invoke("set_wifi_enabled", { enabled: !wifiEnabled });
      wifiEnabled = !wifiEnabled;
      await refresh();
    } catch {}
  }

  function openDetail() {
    openPopover("network");
  }

  const active = $derived(!!status?.connected && !airplane);
  const subtitle = $derived(
    airplane
      ? "Airplane Mode"
      : !wifiEnabled && status?.connection_type !== "ethernet"
        ? "WiFi off"
        : status?.connected
          ? (status.name ?? status.connection_type)
          : "Disconnected",
  );
</script>

<BaseTile
  label="Network"
  statusText={subtitle}
  {active}
  onclick={handleClick}
  onDetail={openDetail}
  detailLabel="Open network picker"
>
  {#snippet icon()}
    {#if airplane}
      <Plane size={16} strokeWidth={1.75} />
    {:else if status?.connection_type === "ethernet" && status.connected}
      <Cable size={16} strokeWidth={1.75} />
    {:else if active}
      <Wifi size={16} strokeWidth={1.75} />
    {:else}
      <WifiOff size={16} strokeWidth={1.75} />
    {/if}
  {/snippet}
</BaseTile>
