<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Recent-actions indicator for the top bar (CAH-4): the entry point to the
  /// unified undo panel. Deliberately quiet - no standing count badge; the
  /// history is a safety net, not a notification stream.
  import { togglePopover, hoverPopover, activePopover } from "$lib/stores/activePopover.js";
  import { Applet } from "@arlen/ui-kit/components/topbar";
  import { Undo2 } from "lucide-svelte";
  import { loadUndoHistory } from "$lib/stores/undoHistory";

  function onclick(): void {
    void loadUndoHistory();
    togglePopover("undo");
  }
</script>

<Applet
  appletId="undo"
  tooltip="Recent actions"
  ariaLabel={$t("sh.undo.indicator")}
  popoverOpen={$activePopover === "undo"}
  {onclick}
  onmouseenter={() => hoverPopover("undo")}
>
  {#snippet icon()}
    <Undo2 size={14} strokeWidth={1.75} />
  {/snippet}
</Applet>
