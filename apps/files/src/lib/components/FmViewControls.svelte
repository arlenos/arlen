<script lang="ts">
  /// The view-level controls in the headerbar: how the focused pane
  /// renders (icon segmented), the dual-pane toggle and the hidden
  /// toggle. Location-level controls stay in the toolbar — this row
  /// is "how I look at it", the toolbar is "where I am".
  import {
    Columns2,
    Columns3,
    Eye,
    EyeOff,
    LayoutGrid,
    List,
  } from "@lucide/svelte";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import type { ViewMode } from "@arlen/ui-kit/components/browser";
  import { focusedController, splitView, toggleSplit } from "$lib/stores/panes";

  const VIEW_OPTIONS = [
    { value: "list", label: "List view", icon: List },
    { value: "grid", label: "Grid view", icon: LayoutGrid },
    { value: "miller", label: "Column view", icon: Columns3 },
  ];

  // Live mirrors of the focused controller's stores.
  let mode = $state<ViewMode>("list");
  let hidden = $state(false);
  $effect(() => {
    const c = $focusedController;
    if (!c) return;
    const u1 = c.viewMode.subscribe((v) => (mode = v));
    const u2 = c.showHidden.subscribe((v) => (hidden = v));
    return () => {
      u1();
      u2();
    };
  });
</script>

{#if $focusedController}
  <div class="view-controls">
    <SegmentedControl
      ariaLabel="View"
      options={VIEW_OPTIONS}
      value={mode}
      onchange={(v) => $focusedController?.viewMode.set(v as ViewMode)}
    />
    <IconAction
      label={$splitView ? "Close the second pane" : "Split into two panes"}
      size="control"
      active={$splitView}
      onclick={() => toggleSplit()}
    >
      <Columns2 size={15} strokeWidth={1.75} />
    </IconAction>
    <IconAction
      label={hidden ? "Hide hidden files" : "Show hidden files"}
      size="control"
      active={hidden}
      onclick={() => $focusedController?.setShowHidden(!hidden)}
    >
      {#if hidden}
        <Eye size={15} strokeWidth={1.75} />
      {:else}
        <EyeOff size={15} strokeWidth={1.75} />
      {/if}
    </IconAction>
  </div>
{/if}

<style>
  .view-controls {
    display: flex;
    align-items: center;
    gap: 4px;
  }
</style>
