<script lang="ts">
  import GlobalMenuBar from "$lib/components/GlobalMenuBar.svelte";
  import ClockIndicator from "$lib/components/ClockIndicator.svelte";
  import NetworkIndicator from "$lib/components/NetworkIndicator.svelte";
  import AudioIndicator from "$lib/components/AudioIndicator.svelte";
  import BatteryIndicator from "$lib/components/BatteryIndicator.svelte";
  import BluetoothIndicator from "$lib/components/BluetoothIndicator.svelte";
  import BluetoothPopover from "$lib/components/BluetoothPopover.svelte";
  import TrayIndicator from "$lib/components/TrayIndicator.svelte";
  import TrayPopover from "$lib/components/TrayPopover.svelte";
  import PanelTrigger from "$lib/components/PanelTrigger.svelte";
  import QuickSettingsPanel from "$lib/components/QuickSettingsPanel.svelte";
  import NetworkPopover from "$lib/components/NetworkPopover.svelte";
  import AudioPopover from "$lib/components/AudioPopover.svelte";
  import BatteryPopover from "$lib/components/BatteryPopover.svelte";
  import WorkspaceIndicator from "$lib/components/WorkspaceIndicator.svelte";
</script>

<!--
  Top Bar Layout (Issue #46):
  [App Menu + Toolbar] -- [Workspaces] -- [Network] [Audio] [Battery] [Panel] [Clock]
       LEFT                  CENTER                        RIGHT
-->
<div class="topbar shell-surface" data-tauri-drag-region>

  <div class="region region-left" data-tauri-drag-region>
    <GlobalMenuBar />
    <div class="slot-toolbar"></div>
  </div>

  <div class="region region-center" data-tauri-drag-region>
    <WorkspaceIndicator />
  </div>

  <div class="region region-right">
    <div class="slot-sni">
      <TrayIndicator />
    </div>
    <div class="slot-project"></div>
    <div class="region-separator"></div>
    <div class="slot-temp"></div>

    <div class="slot-system-icons">
      <NetworkIndicator />
      <BluetoothIndicator />
      <AudioIndicator />
      <BatteryIndicator />
      <div class="system-separator"></div>
      <ClockIndicator />
      <PanelTrigger />
    </div>
  </div>
</div>

<NetworkPopover />
<AudioPopover />
<BatteryPopover />
<BluetoothPopover />
<TrayPopover />
<QuickSettingsPanel />

<style>
  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 36px;
    width: 100%;
    padding: 0 8px;
    gap: 16px;
    position: relative;
    user-select: none;
    flex-shrink: 0;
    background: var(--background);
  }

  .region {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }

  .region-left { flex: 1; }
  .region-center { flex: 0 0 auto; justify-content: center; }
  .region-right { flex: 1; justify-content: flex-end; }

  .slot-toolbar, .slot-sni, .slot-project, .slot-temp {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .slot-toolbar:empty, .slot-sni:empty, .slot-project:empty, .slot-temp:empty {
    display: none;
  }

  .slot-system-icons {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .system-separator {
    width: 1px;
    height: 14px;
    background: color-mix(in srgb, var(--foreground) 15%, transparent);
    margin: 0 4px;
    align-self: center;
    flex-shrink: 0;
  }

  .region-separator {
    width: 1px;
    height: 14px;
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    flex-shrink: 0;
  }

  .slot-project:empty + .region-separator { display: none; }
</style>
