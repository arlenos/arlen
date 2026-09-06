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
  import { t } from "$lib/i18n/messages";

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

  /// True when the adapter state could not be read at all.
  ///
  /// NOT THE SAME AS OFF, and this icon drew them identically: a failed read set
  /// `btState = null`, `powered` collapsed to false, and the bar showed the
  /// slashed glyph with the tooltip "Bluetooth: Off" - a statement about the
  /// adapter, made after nobody could reach it. The network indicator beside it
  /// learnt this on 20 August and says so in its own note: "the slash is a
  /// statement that there is no connection, and this is the state where nobody
  /// knows". Same fix here, one component over.
  let unknown = $state(false);

  async function load() {
    try {
      btState = await invoke<BluetoothState>("get_bluetooth_state");
      unknown = false;
    } catch {
      btState = null;
      unknown = true;
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

  /// Through the catalogue, like the battery indicator beside it. These two
  /// strings were English literals in a bar that is otherwise translated.
  const tooltip = $derived(
    unknown
      ? $t("sh.bt.tip.unknown")
      : !powered
        ? $t("sh.bt.tip.off")
        : primaryDevice
          ? primaryDevice.name +
            (primaryDevice.battery_percentage != null
              ? ` (${$t("sh.pct", { n: primaryDevice.battery_percentage })})`
              : "")
          : $t("sh.bt.tip.plain"),
  );

  const isOpen = $derived($activePopover === "bluetooth");
</script>

{#if visible}
  <Applet
    appletId="bluetooth"
    {tooltip}
    popoverOpen={isOpen}
    dimmed={!powered}
    state={unknown ? "off" : powered ? "on" : "off"}
    onclick={() => togglePopover("bluetooth")}
    onmouseenter={() => hoverPopover("bluetooth")}
  >
    {#snippet icon()}
      <!-- The plain glyph when nobody could ask: the slash SAYS the adapter is
           off, and that is the one thing this state does not know. Dimmed, so it
           does not read as on either. -->
      {#if unknown}
        <Bluetooth size={14} strokeWidth={1.5} />
      {:else if !powered}
        <BluetoothOff size={14} strokeWidth={1.5} />
      {:else}
        <Bluetooth size={14} strokeWidth={1.5} />
      {/if}
    {/snippet}
  </Applet>
{/if}
