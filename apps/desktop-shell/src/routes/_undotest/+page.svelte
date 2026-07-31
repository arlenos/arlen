<script lang="ts">
  /// Headless look-mock for the unified recent-actions panel (CAH-4): the
  /// undo indicator on a topbar strip with the panel open, driven by the
  /// store fixture. `?state=empty` shows the nothing-to-take-back state.
  /// Not shipped in any nav; a dev/test route for the screenshot loop
  /// (the `_mpristest` pattern).
  import { onMount } from "svelte";
  import UndoIndicator from "$lib/components/UndoIndicator.svelte";
  import UndoPopover from "$lib/components/UndoPopover.svelte";
  import { openPopover } from "$lib/stores/activePopover.js";
  import { loadUndoHistory, undoHistory } from "$lib/stores/undoHistory";

  onMount(async () => {
    await loadUndoHistory();
    if (new URLSearchParams(window.location.search).get("state") === "empty") {
      undoHistory.set([]);
    }
    openPopover("undo");
  });
</script>

<div class="stage">
  <div class="strip">
    <span class="strip-label">topbar strip</span>
    <UndoIndicator />
  </div>
  <UndoPopover />
</div>

<style>
  .stage {
    position: relative;
    width: 100vw;
    height: 100vh;
    background: #0a0c10;
  }
  .strip {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 0.5rem;
    height: 2rem;
    padding: 0 7rem 0 1rem;
    background: color-mix(in srgb, #ffffff 4%, transparent);
  }
  .strip-label {
    margin-right: auto;
    font-size: 11px;
    color: color-mix(in srgb, #ffffff 35%, transparent);
  }
</style>
