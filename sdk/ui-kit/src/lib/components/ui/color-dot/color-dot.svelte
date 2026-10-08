<script lang="ts">
  /// A small square of one colour: a calendar's colour, a pen in the
  /// annotator, a series in a chart legend. Three apps drew their own, at
  /// three sizes with three borders and three ways of showing the chosen one,
  /// and one of them named its buttons by their hex value. This is the one.
  ///
  /// Without `onclick` it is a mark beside a label (`legend`, hidden from a
  /// reader, since the label says it). With `onclick` it is a button a person
  /// picks with (`pick`): it needs a `label` naming the colour in words, and
  /// `selected` puts the outline on the chosen one. Focus has its own ring, so
  /// the chosen dot still shows focus. A hairline in the text colour keeps a
  /// colour close to the background visible on either theme.
  let {
    color,
    size = "legend",
    label,
    selected = false,
    id,
    onclick,
  }: {
    /// Any CSS colour.
    color: string;
    /// `legend` (10px, a mark) or `pick` (20px, a target).
    size?: "legend" | "pick";
    /// The colour's name in words; required when the dot is a button.
    label?: string;
    /// The chosen one in a set; only read on a button.
    selected?: boolean;
    id?: string;
    onclick?: () => void;
  } = $props();
</script>

{#if onclick}
  <button
    type="button"
    class="color-dot {size}"
    class:selected
    style:background={color}
    aria-label={label}
    aria-pressed={selected}
    {id}
    {onclick}
  ></button>
{:else}
  <span class="color-dot {size}" style:background={color} aria-hidden="true" {id}></span>
{/if}

<style>
  .color-dot {
    display: inline-block;
    flex-shrink: 0;
    padding: 0;
    border: 1px solid color-mix(in srgb, var(--foreground) 25%, transparent);
    border-radius: var(--radius-chip);
  }
  .legend {
    width: 0.625rem;
    height: 0.625rem;
  }
  .pick {
    width: 1.25rem;
    height: 1.25rem;
  }
  .selected {
    outline: 2px solid var(--color-accent, var(--primary));
    outline-offset: 1px;
  }
  .color-dot:focus-visible {
    box-shadow:
      0 0 0 2px var(--background),
      0 0 0 4px var(--foreground);
  }
</style>
