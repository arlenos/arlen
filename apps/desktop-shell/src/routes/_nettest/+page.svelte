<script lang="ts">
  /// Headless look-mock for the network popover, the `_qstest` pattern.
  ///
  /// The panel lives behind a top-bar trigger that only exists once Tauri is
  /// present, so under plain vite it cannot be reached - and the state worth
  /// looking at is the one nobody could produce on purpose: NetworkManager not
  /// answering. That used to render as a definite "WiFi is off" with a switch
  /// beside it, about a radio nobody had managed to ask.
  ///
  /// `?state=ok|wifi-off|unknown`, `?panel=bluetooth`, `?locale=de`. Not in any nav.
  import { onMount } from "svelte";
  import NetworkPopover from "$lib/components/NetworkPopover.svelte";
  import BluetoothPopover from "$lib/components/BluetoothPopover.svelte";
  import { openPopover } from "$lib/stores/activePopover.js";
  import { locale } from "@arlen/ui-kit/i18n";

  const params = typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;
  if (params?.get("locale")) locale.set(params.get("locale") as string);
  const pinned = params?.get("state") ?? "ok";
  const panel = params?.get("panel") === "bluetooth" ? "bluetooth" : "network";

  let ready = $state(false);

  onMount(async () => {
    const { mockIPC } = await import("@tauri-apps/api/mocks");
    mockIPC((cmd) => {
      // `unknown` answers nothing at all, which is what a stopped
      // NetworkManager looks like from here. The other two answer normally so
      // the honest states can be compared against the definite ones.
      if (pinned === "unknown") throw new Error("the service is not running");
      if (cmd === "get_bluetooth_state")
        return {
          available: true,
          powered: true,
          discovering: false,
          devices: [
            { path: "/d1", name: "WH-1000XM4", connected: true, paired: true, icon: "audio-headphones", battery: 80 },
          ],
        };
      if (cmd === "get_airplane_mode") return false;
      if (cmd === "get_wifi_enabled") return pinned !== "wifi-off";
      if (cmd === "get_network_status")
        return {
          connection_type: "wifi",
          connected: true,
          name: "Kicker",
          signal_strength: 72,
          vpn_active: false,
        };
      if (cmd === "get_wifi_networks")
        return [
          { ssid: "Kicker", signal: 72, security: "WPA2", is_connected: true, is_known: true },
          { ssid: "Nachbar 2.4", signal: 41, security: "WPA2", is_connected: false, is_known: false },
        ];
      if (cmd === "get_vpn_connections") return [];
      return null;
    });
    ready = true;
    openPopover(panel);
  });
</script>

<div class="wrap">
  {#if ready}
    {#if panel === "bluetooth"}
      <BluetoothPopover />
    {:else}
      <NetworkPopover />
    {/if}
  {/if}
</div>

<style>
  .wrap {
    min-height: 100vh;
    background: #0a0a0a;
  }
</style>
