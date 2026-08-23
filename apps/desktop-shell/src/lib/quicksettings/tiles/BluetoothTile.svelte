<script lang="ts">
  import { shellRead } from "$lib/shellRead";
  import { t } from "$lib/i18n/messages";
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
  import { shellAction } from "$lib/shellAction";
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
    const got = await shellRead<BluetoothState>("get_bluetooth_state", "bluetooth-tile");
    if (got !== null) state = got;
  }

  async function handleClick() {
    await shellAction(
      "set_bluetooth_powered",
      { enabled: !state.powered },
      "sh.tile.errBluetooth",
    );
    await refresh();
  }

  function openDetail() {
    openPopover("bluetooth");
  }

  const connected = $derived(state.devices.find((d) => d.connected));
  const subtitle = $derived(
    !state.powered ? $t("sh.tile.btOff") : connected ? connected.name : $t("sh.tile.btNoDevice"),
  );
</script>

{#if state.available}
  <BaseTile
    label={$t("sh.tile.bluetooth")}
    statusText={subtitle}
    active={state.powered}
    onclick={handleClick}
    onDetail={openDetail}
    detailLabel={$t("sh.tile.btDetail")}
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
