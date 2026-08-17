<script lang="ts">
  /// Headless look-mock for the network popover, the `_qstest` pattern.
  ///
  /// The panel lives behind a top-bar trigger that only exists once Tauri is
  /// present, so under plain vite it cannot be reached - and the state worth
  /// looking at is the one nobody could produce on purpose: NetworkManager not
  /// answering. That used to render as a definite "WiFi is off" with a switch
  /// beside it, about a radio nobody had managed to ask.
  ///
  /// `?state=ok|wifi-off|unknown|refuse-write`, `?panel=bluetooth|audio|tray|battery`,
  /// `?locale=de`. Not in any nav.
  ///
  /// `refuse-write` answers every read and refuses the writes a PERSON makes, so
  /// a panel is fully drawn and only the pressed control fails - the one state
  /// nothing else can produce, since these panels need Tauri to open at all.
  ///
  /// It refuses a NAMED list rather than everything starting with `set_`, and
  /// that is the whole lesson of building it. The obvious spelling blanked the
  /// panel completely, and recording what the mock actually saw said why: the
  /// first command through is `set_popover_input_region`, the shell reshaping
  /// its own compositor input region as the popover opens. Refusing that refuses
  /// the popover. `set_` names both a user's intent and the window plumbing
  /// underneath it, so only the intents belong here.
  import { onMount } from "svelte";
  import NetworkPopover from "$lib/components/NetworkPopover.svelte";
  import BluetoothPopover from "$lib/components/BluetoothPopover.svelte";
  import AudioPopover from "$lib/components/AudioPopover.svelte";
  import TrayPopover from "$lib/components/TrayPopover.svelte";
  import BatteryPopover from "$lib/components/BatteryPopover.svelte";
  import { openPopover } from "$lib/stores/activePopover.js";
  import { locale } from "@arlen/ui-kit/i18n";

  const params = typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;
  if (params?.get("locale")) locale.set(params.get("locale") as string);
  const pinned = params?.get("state") ?? "ok";
  const requested = params?.get("panel");
  const panel =
    requested === "bluetooth"
      ? "bluetooth"
      : requested === "audio"
        ? "audio"
        : requested === "tray"
          ? "tray"
          : requested === "battery"
            ? "battery"
            : "network";

  /// The commands a person's press sends, as opposed to the ones the shell sends
  /// itself. Add to this when a panel grows a control worth watching refuse.
  /// Named per command rather than by prefix, and the audio ones are why the
  /// list is worth keeping honest: `toggle_audio_mute` does not start with
  /// `set_`, so a first version of this quietly let the mute press SUCCEED and
  /// the panel had nothing to report. I read that as a missing refusal in the
  /// sound panel until I looked at its code, where every failure sets an error.
  /// A gap in the harness reads exactly like a gap in the app.
  const USER_WRITES = [
    "set_power_profile",
    "set_wifi_enabled",
    "set_airplane_mode",
    "set_bluetooth_powered",
    "toggle_audio_mute",
    "toggle_input_mute",
    "set_audio_volume",
    "set_input_volume",
    "set_audio_output",
    "set_audio_input",
    "set_app_volume",
  ];

  let ready = $state(false);

  onMount(async () => {
    const { mockIPC } = await import("@tauri-apps/api/mocks");
    mockIPC((cmd) => {
      // `unknown` answers nothing at all, which is what a stopped
      // NetworkManager looks like from here. The other two answer normally so
      // the honest states can be compared against the definite ones.
      if (pinned === "unknown") throw new Error("the service is not running");
      if (pinned === "refuse-write" && USER_WRITES.includes(cmd as string))
        throw new Error("the service refused that");
      if (cmd === "get_battery_status")
        return {
          percentage: 62,
          state: "discharging",
          time_remaining: 8100,
          icon_name: "battery-good",
        };
      if (cmd === "get_power_profile") return "balanced";
      if (cmd === "get_sni_items")
        // A tray item's title is the APP's own name, reported by that app over
        // StatusNotifierItem - never ours to translate. The fixture uses a real
        // one so the row is the width a real one would be; it sits in the i18n
        // baseline as declared foreign data for the same reason the live titles
        // are not translated either.
        return [
          {
            service: "s1",
            id: "Vesktop",
            title: "Vesktop",
            status: "Active",
            icon_name: null,
            icon_pixmap: null,
            tooltip_title: null,
            tooltip_description: null,
            menu_path: null,
          },
        ];
      if (cmd === "get_audio_full_state")
        return {
          status: { volume: 62, muted: false, output_type: "speaker" },
          input_status: { volume: 40, muted: true },
          outputs: [{ id: "o1", name: "Built-in Speakers", is_default: true }],
          inputs: [{ id: "i1", name: "Built-in Microphone", is_default: true }],
          apps: [],
        };
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
    {:else if panel === "audio"}
      <AudioPopover />
    {:else if panel === "tray"}
      <TrayPopover />
    {:else if panel === "battery"}
      <BatteryPopover />
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
