<script lang="ts">
  /// The one shared frame for the capture and note views: a fixed head row, the
  /// centred content column, and the transcript rail. Replaces the two
  /// copy-pasted full-height shells; the layout's main is the only scroller
  /// above this, and each region here scrolls itself.
  import type { Snippet } from "svelte";

  let {
    head,
    content,
    rail,
  }: {
    head: Snippet;
    content: Snippet;
    rail?: Snippet;
  } = $props();
</script>

<div class="shell">
  <div class="shell-head">
    {@render head()}
  </div>
  <div class="shell-body" class:with-rail={!!rail}>
    <div class="shell-content">
      <div class="shell-column">
        {@render content()}
      </div>
    </div>
    {#if rail}
      {@render rail()}
    {/if}
  </div>
</div>

<style>
  .shell {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .shell-head {
    flex-shrink: 0;
    padding: 0.9rem 1.5rem;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
  }
  .shell-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .shell-content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  /* The content column centres in ITS pane (not the window), so it lines up
     whether or not the rail is there - the old dead-gap asymmetry is gone. */
  .shell-column {
    width: 100%;
    max-width: 44rem;
    margin: 0 auto;
    padding: 1.5rem;
  }
  .shell-body.with-rail > :global(aside) {
    flex: 0 0 24rem;
    min-height: 0;
  }
  /* THEY STACK WHERE THEY CANNOT SIT SIDE BY SIDE, and what forced this is the
     worst starved column measured today. The rail is `flex: 0 0 24rem` - 384px
     that never shrink - beside a content column that does. At 720, with the app
     sidebar taking its share, the body is about 470: the rail kept all 384 and
     the person's OWN NOTES were left 22px, rendering one word per line down a
     ribbon ("why / build / our / own / editor:") and further down single
     letters stacked vertically.
     
     Below 60rem the transcript goes under the notes instead. It is the
     verification source and the notes are the thing being verified, so if only
     one can be full width it is not the rail; and stacked, both keep their own
     scroll rather than one being reduced to a column of letters. */
  @media (max-width: 60rem) {
    .shell-body.with-rail {
      flex-direction: column;
    }
    .shell-body.with-rail > :global(aside) {
      flex: 1 1 40%;
      border-inline-start: none;
      border-top: 1px solid color-mix(in srgb, var(--color-fg-primary) 8%, transparent);
    }
  }
</style>
