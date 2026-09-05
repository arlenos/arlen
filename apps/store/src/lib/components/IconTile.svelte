<script lang="ts">
  /// The one place that decides how an icon reference is painted. The wire
  /// sends a URL or a bare theme name, and the fixture paints CSS gradients.
  /// Anything unpaintable falls back to a monogram rather than a blank tile -
  /// a blank square reads as a rendering bug, a letter reads as an app without
  /// an icon, and only one of those is true.
  ///
  /// `icon://` IS THE LOCAL DELIVERY THIS DOC USED TO CALL A ROUTED SEAM. The
  /// catalogue's icons are files staged on the machine, and a webview cannot
  /// open a path, so `apps/store/src-tauri/src/icon_scheme.rs` serves them over
  /// a scheme of its own and rewrites the path into that URL before a card
  /// leaves for the frontend. Until the route existed every one of the 2531
  /// Debian components fell to a monogram. The scheme is `icon://` on Linux and
  /// macOS, which is the shape tauri gives a custom protocol there.
  let {
    icon,
    name,
    size = "3rem",
  }: { icon: string | null; name: string; size?: string } = $props();

  let broken = $state(false);
  const mode = $derived(
    icon?.startsWith("linear-gradient(")
      ? "css"
      : icon && /^(https?|icon):\/\//.test(icon) && !broken
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
