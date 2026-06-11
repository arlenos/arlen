<script lang="ts">
  /// An image from command output (Kitty/iTerm graphics or a
  /// generated chart): rendered for real, pixel-inert, inside the
  /// output frame. `src` is whatever the backend stamped — the
  /// element can only paint it.
  let {
    src,
    alt,
    width = null,
    height = null,
  }: {
    src: string;
    alt: string;
    width?: number | null;
    height?: number | null;
  } = $props();
</script>

<img
  class="image-block"
  {src}
  {alt}
  width={width ?? undefined}
  height={height ?? undefined}
  draggable="false"
/>

<style>
  .image-block {
    display: block;
    max-width: 100%;
    height: auto;
    border-radius: var(--radius-chip);
  }
</style>
