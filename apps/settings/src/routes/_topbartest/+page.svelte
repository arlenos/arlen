<script lang="ts">
  /// Headless render harness for the Topbar settings panel. UI-AFFORDANCE
  /// verification ONLY, NOT a behaviour claim. Mocks the inventory command
  /// (`topbar_items`) + the config writes, then renders the REAL panel so the
  /// preview, the sortable rows + drag handles, the shown/overflow switches, and
  /// the tray-tagged rows are screenshot-verifiable. The drag is driven with a
  /// real pointer in the loop. The live bar re-render is the coder's. Dev route.
  import { onMount } from "svelte";
  import TopbarPanel from "../topbar/+page.svelte";

  const SEED = [
    { id: "notifications", name: "Notifications", icon: "notifications", kind: "applet", shown: true },
    { id: "audio", name: "Audio", icon: "audio", kind: "applet", shown: true },
    { id: "network", name: "Network", icon: "network", kind: "applet", shown: true },
    { id: "bluetooth", name: "Bluetooth", icon: "bluetooth", kind: "applet", shown: false },
    { id: "battery", name: "Battery", icon: "battery", kind: "applet", shown: true },
    { id: "clock", name: "Clock", icon: "clock", kind: "applet", shown: true },
    { id: "quick-settings", name: "Quick settings", icon: "quick-settings", kind: "applet", shown: true },
    { id: "sni-syncthing", name: "Syncthing", icon: "tray", kind: "tray", shown: false },
  ];

  let ready = $state(false);
  onMount(async () => {
    const { mockIPC } = await import("@tauri-apps/api/mocks");
    mockIPC((cmd) => {
      if (cmd === "topbar_items") return SEED;
      return null;
    });
    ready = true;
  });
</script>

{#if ready}
  <TopbarPanel />
{/if}
