<script lang="ts">
  /// Bluetooth indicator for the top bar.
  ///
  /// Always visible when an adapter exists and is powered. Shows the icon
  /// of the highest-priority connected device, or generic Bluetooth icon.

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { togglePopover, hoverPopover } from "$lib/stores/activePopover.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
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
    return () => { unlisten.then((fn) => fn()); };
  });

  const connectedDevices = $derived(
    btState?.devices.filter((d: BluetoothDevice) => d.connected) ?? []
  );

  // Visible whenever hardware exists (even if powered off or errored).
  // Only hidden when no adapter is detected at all.
  const visible = $derived(btState === null || btState.available);

  const powered = $derived(btState?.powered ?? false);

  const primaryDevice = $derived(
    connectedDevices.find((d: BluetoothDevice) => d.icon.includes("audio") || d.icon.includes("headset")) ??
    connectedDevices.find((d: BluetoothDevice) => d.icon.includes("input")) ??
    connectedDevices[0] ??
    null
  );

  const label = $derived(
    !powered ? "Bluetooth: Off" :
    primaryDevice
      ? primaryDevice.name + (primaryDevice.battery_percentage != null ? ` (${primaryDevice.battery_percentage}%)` : "")
      : "Bluetooth"
  );
</script>

{#if visible}
  <Tooltip.Root>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        <button
          {...props}
          class="bt-btn"
          class:off={!powered}
          aria-label={label}
          onclick={() => togglePopover("bluetooth")}
          onmouseenter={() => hoverPopover("bluetooth")}
        >
          {#if !powered}
            <BluetoothOff size={14} strokeWidth={1.5} />
          {:else}
            <Bluetooth size={14} strokeWidth={1.5} />
          {/if}
        </button>
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Content>{label}</Tooltip.Content>
  </Tooltip.Root>
{/if}

<style>
  .bt-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--foreground);
    transition: background-color 150ms ease;
  }
  .bt-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .bt-btn.off {
    opacity: 0.4;
  }
</style>
