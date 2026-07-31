<script lang="ts">
  /// Headless look-mock for the Activity/Jobs zone: the zone on a popover-width
  /// card, driven by its store fixture. Not shipped in any nav; a dev/test
  /// route for the screenshot loop (the `_undotest` pattern).
  import ActivityJobsZone from "$lib/components/ActivityJobsZone.svelte";
</script>

<div class="stage">
  <div class="card">
    <ActivityJobsZone />
  </div>
</div>

<style>
  .stage {
    width: 100vw;
    height: 100vh;
    display: flex;
    align-items: flex-start;
    justify-content: center;
    padding-top: 3rem;
    background: #0a0c10;
  }
  .card {
    width: 380px;
    border: 1px solid color-mix(in srgb, #ffffff 10%, transparent);
    border-radius: var(--radius-card, 12px);
    background: var(--color-bg-card, #14161c);
    padding: 12px;
  }
</style>
