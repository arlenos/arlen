<script lang="ts">
  /// The one place that decides how an icon reference is painted. The wire
  /// sends a URL, a local path or a bare theme name; only a URL loads in the
  /// webview today (local icon delivery is a routed seam), and the fixture
  /// paints CSS gradients. Anything unpaintable falls back to a monogram
  /// rather than a blank tile - a blank square reads as a rendering bug, a
  /// letter reads as an app without an icon, and only one of those is true.
  let {
    icon,
    name,
    size = "3rem",
  }: { icon: string | null; name: string; size?: string } = $props();

  let broken = $state(false);
  const mode = $derived(
    icon?.startsWith("linear-gradient(")
      ? "css"
      : icon && /^https?:\/\//.test(icon) && !broken
        ? "img"
        : "monogram",
  );
  const letter = $derived((name.trim()[0] ?? "?").toUpperCase());
</script>

<span class="tile" style="width: {size}; height: {size}; --tile-size: {size};" aria-hidden="true">
  {#if mode === "css"}
    <span class="fill" style="background: {icon}"></span>
  {:else if mode === "img"}
    <img src={icon} alt="" loading="lazy" onerror={() => (broken = true)} />
  {:else}
    <span class="mono">{letter}</span>
  {/if}
</span>

<style>
  .tile {
    display: block;
    flex-shrink: 0;
    overflow: hidden;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-fg-primary) 6%, transparent);
  }
  .fill {
    display: block;
    width: 100%;
    height: 100%;
  }
  img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .mono {
    display: flex;
    width: 100%;
    height: 100%;
    align-items: center;
    justify-content: center;
    font-size: calc(var(--tile-size) * 0.42);
    font-weight: 600;
    color: color-mix(in srgb, var(--color-fg-primary) 45%, transparent);
  }
</style>
