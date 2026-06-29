<script lang="ts">
  /// Top-bar trigger for the Quick Settings panel.
  ///
  /// Wraps the shared `Applet` primitive. The unread badge that
  /// used to live here moved to NotificationsTrigger; this is
  /// purely the QS-panel toggle.
  import {
    activePopover,
    togglePopover,
    hoverPopover,
  } from "$lib/stores/activePopover.js";
  import { Applet } from "@arlen/ui-kit/components/topbar";
  import { Square } from "lucide-svelte";

  const isOpen = $derived($activePopover === "quick-settings");
</script>

<Applet
  appletId="quick-settings"
  tooltip="Quick Settings"
  popoverOpen={isOpen}
  onclick={() => togglePopover("quick-settings")}
  onmouseenter={() => hoverPopover("quick-settings")}
>
  {#snippet icon()}
    <Square size={14} strokeWidth={1.5} />
  {/snippet}
</Applet>
