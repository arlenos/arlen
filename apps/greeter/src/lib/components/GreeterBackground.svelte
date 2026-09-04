<script lang="ts">
  /// The fullscreen background: the wallpaper, exactly as the desktop shows
  /// it behind the (transparent) shell. A single flat, even dim sits over it
  /// so the panel and clock read on any wallpaper. No gradient scrim, no
  /// blur. High contrast drops the image for a flat black field. `image` is
  /// the system wallpaper inlined by `greeter_wallpaper`; it may be null, and
  /// the fallback below is what a login with no wallpaper shows.
  let {
    image = null,
    highContrast = false,
  }: {
    image?: string | null;
    highContrast?: boolean;
  } = $props();
</script>

<div class="bg" class:hc={highContrast} aria-hidden="true">
  {#if !highContrast && image}
    <div class="photo" style="background-image: url('{image}')"></div>
    <div class="dim"></div>
  {/if}
</div>

<style>
  .bg {
    position: fixed;
    inset: 0;
    z-index: 0;
    /* The fallback when no wallpaper is set: the desktop's own near-black. */
    background: var(--color-bg-shell, #0a0a0a);
  }
  .bg.hc {
    background: #000000;
  }
  .photo {
    position: absolute;
    inset: 0;
    background-size: cover;
    background-position: center;
  }
  /* One flat, even darkening. Not a gradient: the panel carries the auth, so
     this only takes the wallpaper down a notch for the clock and corners. */
  .dim {
    position: absolute;
    inset: 0;
    background: var(--greeter-dim);
  }
</style>
