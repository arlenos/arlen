<script lang="ts">
  /// Dev route for the a11y keyboard walkthrough: mounts the same fixture the
  /// axe gate renders, so focus order, focus rings and arrow-key operation can
  /// be eyeballed (Tab through, Arrow on the slider/choice/colour pad). Not in
  /// any nav; a harness for the a11y pass.
  import Kitchen from "$lib/components/a11y-kitchen.svelte";
</script>

<div class="page">
  <Kitchen />
</div>

<style>
  :global(body) {
    margin: 0;
  }
  .page {
    min-height: 100vh;
    padding: 24px;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-app, #fafafa);
  }
  .page :global(section) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 16px;
    margin: 0 0 20px;
  }
  .page :global(h1) {
    font-size: 1rem;
  }
</style>
