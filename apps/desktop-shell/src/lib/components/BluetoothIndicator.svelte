<script lang="ts">
  /// Bluetooth indicator for the top bar.
  ///
  /// Wraps the shared `Applet` primitive. Visible whenever an
  /// adapter exists (even if powered off). Dims when the adapter
  /// is off; shows the connected device's name (+ battery if known)
  /// in the tooltip when something is connected.

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { togglePopover, hoverPopover, activePopover } from "$lib/stores/activePopover.js";
  import { Applet } from "@arlen/ui-kit/components/topbar";
  import { Bluetooth, BluetoothOff } from "lucide-svelte";

  interface BluetoothDevice {
    path: string;
    address: string;
    name: string;
    icon: string;
    paired: boolean;
    connected: boolean;
    trusted: boolean;
    battery_percentage: number | null;
  }

  interface BluetoothState {
    available: boolean;
    powered: boolean;
    discovering: boolean;
    devices: BluetoothDevice[];
  }

  let btState = $state<BluetoothState | null>(null);

  async function load() {
    try {
      btState = await invoke<BluetoothState>("get_bluetooth_state");
    } catch {
      btState = null;
    }
  }

  onMount(() => {
    load();
    const unlisten = listen("bluetooth-changed", () => load());
    return () => {
      unlisten.then((fn) => fn());
    };
  });

  const connectedDevices = $derived(
    btState?.devices.filter((d: BluetoothDevice) => d.connected) ?? [],
  );

  /// Visible whenever hardware exists (even powered-off / errored).
  /// Hidden only when no adapter at all is detected — keeps the
  /// applet from appearing on desktop machines that physically
  /// have no Bluetooth.
  const visible = $derived(btState === null || btState.available);
  const powered = $derived(btState?.powered ?? false);

  const primaryDevice = $derived(
    connectedDevices.find(
      (d: BluetoothDevice) =>
        d.icon.includes("audio") || d.icon.includes("headset"),
    ) ??
      connectedDevices.find((d: BluetoothDevice) => d.icon.includes("input")) ??
      connectedDevices[0] ??
      null,
  );

  const tooltip = $derived(
    !powered
      ? "Bluetooth: Off"
      : primaryDevice
        ? primaryDevice.name +
          (primaryDevice.battery_percentage != null
            ? ` (${primaryDevice.battery_percentage}%)`
            : "")
        : "Bluetooth",
  );

  const isOpen = $derived($activePopover === "bluetooth");
</script>

{#if visible}
  <Applet
    appletId="bluetooth"
    {tooltip}
    popoverOpen={isOpen}
    dimmed={!powered}
    state={powered ? "on" : "off"}
    onclick={() => togglePopover("bluetooth")}
    onmouseenter={() => hoverPopover("bluetooth")}
  >
    {#snippet icon()}
      {#if !powered}
        <BluetoothOff size={14} strokeWidth={1.5} />
      {:else}
        <Bluetooth size={14} strokeWidth={1.5} />
      {/if}
    {/snippet}
  </Applet>
{/if}
