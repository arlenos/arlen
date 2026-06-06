<script lang="ts">
  /// QS tile: Bluetooth adapter + connected device.
  ///
  /// Single-click toggles the adapter on/off. Right-click opens the
  /// BluetoothPopover for the device list, scan, pair, connect.
  ///
  /// `available_when = "bluetooth-adapter"`: the orchestrator hides
  /// the tile entirely when no BlueZ adapter exists.
  import { BaseTile } from "@arlen/ui-kit/components/quicksettings";
  import { Bluetooth, BluetoothOff } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { openPopover } from "$lib/stores/activePopover.js";

  interface BluetoothDevice {
    address: string;
    name: string;
    connected: boolean;
  }
  interface BluetoothState {
    available: boolean;
    powered: boolean;
    discovering: boolean;
    devices: BluetoothDevice[];
  }

  let state = $state<BluetoothState>({
    available: false,
    powered: false,
    discovering: false,
    devices: [],
  });

  onMount(() => {
    refresh();
    let stop: UnlistenFn | null = null;
    listen("arlen://bluetooth-changed", refresh).then((u) => (stop = u));
    return () => stop?.();
  });

  async function refresh() {
    try {
      state = await invoke<BluetoothState>("get_bluetooth_state");
    } catch {}
  }

  async function handleClick() {
    try {
      await invoke("set_bluetooth_powered", { enabled: !state.powered });
      await refresh();
    } catch {}
  }

  function openDetail() {
    openPopover("bluetooth");
  }

  const connected = $derived(state.devices.find((d) => d.connected));
  const subtitle = $derived(
    !state.powered ? "Off" : connected ? connected.name : "No device",
  );
</script>

{#if state.available}
  <BaseTile
    label="Bluetooth"
    statusText={subtitle}
    active={state.powered}
    onclick={handleClick}
    onDetail={openDetail}
    detailLabel="Open Bluetooth devices"
  >
    {#snippet icon()}
      {#if state.powered}
        <Bluetooth size={16} strokeWidth={1.75} />
      {:else}
        <BluetoothOff size={16} strokeWidth={1.75} />
      {/if}
    {/snippet}
  </BaseTile>
{/if}
