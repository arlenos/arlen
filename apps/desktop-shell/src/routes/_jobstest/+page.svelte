<script lang="ts">
  /// Headless look-mock for the Activity/Jobs zone: the zone on a popover-width
  /// card, driven by its store fixture. Not shipped in any nav; a dev/test
  /// route for the screenshot loop (the `_undotest` pattern).
  import ActivityJobsZone from "$lib/components/ActivityJobsZone.svelte";
</script>

  <!-- `main` + a heading, like `_toasttest` beside it. A look-mock is a
       document a person reads, so the landmark and the level-one heading are
       real here - unlike the shell's own bar, where a `main` would be a
       landmark over nothing. Added when these routes first went into the axe
       table on 6 September and reported the absence. -->
<main class="stage">
  <h1 class="mock-title">Activity and jobs zone</h1>
  <div class="card">
    <ActivityJobsZone />
  </div>
</main>

<style>
  .mock-title {
    margin: 0 0 1rem;
    color: var(--foreground);
    font-size: 1rem;
    font-weight: 600;
  }
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
