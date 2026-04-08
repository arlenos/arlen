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
  import ModuleIndicatorSlot from "$lib/components/ModuleIndicatorSlot.svelte";
  import LayoutIndicator from "$lib/components/LayoutIndicator.svelte";
  import LayoutPopover from "$lib/components/LayoutPopover.svelte";
  import { Separator } from "$lib/components/ui/separator/index.js";
</script>

<!--
  Top Bar Layout:
  [GlobalMenuBar][slot-toolbar]  |  [WorkspaceIndicator]  |  [SNI][slot-project]|[slot-temp][Net BT Audio Bat | Clock Panel]
       region-left                      region-center                           region-right

  Slots: slot-toolbar, slot-sni, slot-project, slot-temp, slot-system-icons
  Empty slots collapse via :empty pseudo-class.
-->
<div
  class="flex items-center justify-between h-9 w-full px-2 gap-4 relative select-none shrink-0 z-50 shell-surface"
  style="background: var(--background)"
  data-tauri-drag-region
>
  <!-- LEFT: App menu + toolbar -->
  <div class="flex items-center gap-2 flex-1 min-w-0" data-tauri-drag-region>
    <GlobalMenuBar />
    <div class="slot-toolbar flex items-center gap-2"></div>
  </div>

  <!-- CENTER: Workspace indicator -->
  <div class="flex-none flex items-center justify-center" data-tauri-drag-region>
    <WorkspaceIndicator />
  </div>

  <!-- RIGHT: Tray + indicators + clock + panel -->
  <div class="flex items-center gap-2 flex-1 justify-end">
    <!-- SNI system tray -->
    <div class="slot-sni flex items-center gap-2">
      <TrayIndicator />
    </div>

    <!-- Focus mode project name (future) -->
    <div class="slot-project flex items-center gap-2"></div>

    <!-- Region separator (hidden when slot-project is empty) -->
    <div class="region-sep"></div>

    <!-- Third-party module indicators -->
    <div class="slot-temp flex items-center gap-0.5">
      <ModuleIndicatorSlot />
    </div>

    <!-- System indicators -->
    <div class="flex items-center gap-0.5">
      <LayoutIndicator />
      <NetworkIndicator />
      <BluetoothIndicator />
      <AudioIndicator />
      <BatteryIndicator />
      <Separator orientation="vertical" class="mx-1 h-3.5 opacity-15" />
      <ClockIndicator />
      <PanelTrigger />
    </div>
  </div>
</div>

<!-- Popovers (rendered outside the bar, positioned fixed) -->
<LayoutPopover />
<NetworkPopover />
<AudioPopover />
<BatteryPopover />
<BluetoothPopover />
<TrayPopover />
<QuickSettingsPanel />

<style>
  /* Empty slots collapse */
  .slot-toolbar:empty,
  .slot-sni:empty,
  .slot-project:empty,
  .slot-temp:empty {
    display: none;
  }

  /* Region separator hides when adjacent slot-project is empty */
  .slot-project:empty + .region-sep {
    display: none;
  }

  .region-sep {
    width: 1px;
    height: 14px;
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    flex-shrink: 0;
  }
</style>
