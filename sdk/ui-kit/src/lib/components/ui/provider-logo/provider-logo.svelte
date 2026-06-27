<script lang="ts">
  /// A provider's brand mark in a small rounded tile. Renders the vendored
  /// monochrome glyph (in the foreground colour, so it sits in Arlen's flat
  /// house style) when one exists for the id, otherwise the provider initial -
  /// the permanent fallback for a custom or local provider with no brand
  /// asset. The id is the catalogue provider id (e.g. `ollama-default`,
  /// `anthropic`); matching is normalised in `providerMark`.
  import { providerMark } from "./logos";

  let {
    id,
    name = id,
    size = 24,
  }: {
    /// The catalogue provider id, used to look up the mark.
    id: string;
    /// The display name; its initial is the fallback glyph.
    name?: string;
    /// Tile size in pixels.
    size?: number;
  } = $props();

  const mark = $derived(providerMark(id));
  const initial = $derived((name || id || "?").charAt(0).toUpperCase());
</script>

<span
  class="provider-logo"
  style="--logo-size: {size}px;"
  aria-hidden="true"
>
  {#if mark}
    <!-- Trusted vendored brand glyph (build-time constant, never user input). -->
    <svg viewBox="0 0 24 24" fill="currentColor" class="mark">{@html mark}</svg>
  {:else}
    <span class="initial">{initial}</span>
  {/if}
</span>

<style>
  .provider-logo {
    flex-shrink: 0;
    width: var(--logo-size, 24px);
    height: var(--logo-size, 24px);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: color-mix(in srgb, var(--foreground) 75%, transparent);
    overflow: hidden;
  }
  /* The glyph sits a touch inside the tile so it reads as a logo, not a
     full-bleed fill. */
  .mark {
    width: 68%;
    height: 68%;
  }
  .initial {
    font-size: calc(var(--logo-size, 24px) * 0.42);
    font-weight: 600;
    line-height: 1;
  }
</style>
