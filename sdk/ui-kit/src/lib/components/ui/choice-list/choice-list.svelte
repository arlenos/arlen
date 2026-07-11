<script lang="ts" module>
  export interface ChoiceOption {
    value: string;
    label: string;
    /// The quiet second line explaining what the choice means.
    description?: string;
    /// An optional caveat line, shown only when this option is selected (e.g.
    /// a dependency the choice relies on).
    note?: string;
  }
</script>

<script lang="ts">
  /// A vertical single-select: each choice on its own row with a label and a
  /// quiet description, so the trade-off of every option is visible at once
  /// (unlike a segmented control, which hides all but the selected hint).
  /// Built on the kit's container-of-flat-items pattern (the PopoverSelect
  /// menu): one bordered surface, borderless rows whose corners are concentric
  /// to it, a foreground wash on hover and the selected row. Flat house style.
  let {
    value,
    options,
    ariaLabel,
    onchange,
  }: {
    value: string;
    options: ChoiceOption[];
    ariaLabel?: string;
    onchange: (value: string) => void;
  } = $props();

  // Radio refs for roving-focus keyboard navigation (the WAI-ARIA radio-group
  // pattern, shared with SegmentedControl).
  let radios = $state<HTMLButtonElement[]>([]);

  // Exactly one radio is tabbable: the selected one, or the first when nothing
  // matches yet, so Tab lands on the group as a single stop.
  const tabbableIndex = $derived.by(() => {
    const i = options.findIndex((o) => o.value === value);
    return i >= 0 ? i : 0;
  });

  function select(v: string) {
    if (v === value) return;
    onchange(v);
  }

  /// Arrow/Home/End move focus AND selection (selection follows focus),
  /// wrapping at the ends.
  function onKeydown(e: KeyboardEvent, i: number) {
    let next = i;
    switch (e.key) {
      case "ArrowDown":
      case "ArrowRight":
        next = (i + 1) % options.length;
        break;
      case "ArrowUp":
      case "ArrowLeft":
        next = (i - 1 + options.length) % options.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = options.length - 1;
        break;
      default:
        return;
    }
    e.preventDefault();
    select(options[next].value);
    radios[next]?.focus();
  }
</script>

<div class="choice-list" role="radiogroup" aria-label={ariaLabel}>
  {#each options as opt, i (opt.value)}
    {@const selected = opt.value === value}
    <button
      bind:this={radios[i]}
      type="button"
      class="choice"
      class:selected
      role="radio"
      aria-checked={selected}
      tabindex={i === tabbableIndex ? 0 : -1}
      onclick={() => select(opt.value)}
      onkeydown={(e) => onKeydown(e, i)}
    >
      <span class="radio" aria-hidden="true"></span>
      <span class="body">
        <span class="label">{opt.label}</span>
        {#if opt.description}
          <span class="desc">{opt.description}</span>
        {/if}
        {#if selected && opt.note}
          <span class="note">{opt.note}</span>
        {/if}
      </span>
    </button>
  {/each}
</div>

<style>
  /* The surface: one bordered container that sets the concentric radius
     context for its rows (the PopoverSelect-menu convention). */
  .choice-list {
    display: flex;
    flex-direction: column;
    width: 100%;
    padding: 4px;
    border-radius: var(--radius-input);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    background: color-mix(in srgb, var(--foreground) 3%, transparent);
    --container-radius: var(--radius-input);
    --container-inset: 4px;
  }
  /* Rows: borderless, transparent, corners concentric to the container. */
  .choice {
    display: flex;
    align-items: flex-start;
    gap: 0.625rem;
    width: 100%;
    padding: 0.5rem 0.625rem;
    border: none;
    background: transparent;
    border-radius: max(0px, calc(var(--container-radius) - var(--container-inset)));
    text-align: start;
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .choice:hover {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }
  .choice.selected {
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
  }
  /* The selection indicator: a hollow box that fills on selection (monochrome).
     Its corners hang off --radius-chip (the smallest-control token), so it
     follows the Roundness slider like the rest of the system instead of a
     hardcoded circle. */
  .radio {
    flex-shrink: 0;
    width: 14px;
    height: 14px;
    margin-top: 0.125rem;
    border-radius: var(--radius-chip);
    border: 1.5px solid color-mix(in srgb, var(--foreground) 30%, transparent);
    transition:
      border-color var(--duration-fast, 150ms) var(--ease-out, ease),
      box-shadow var(--duration-fast, 150ms) var(--ease-out, ease);
  }
  .choice.selected .radio {
    border-color: var(--foreground);
    box-shadow: inset 0 0 0 3px var(--foreground);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    min-width: 0;
  }
  .label {
    font-size: var(--text-sm);
    font-weight: 500;
    color: var(--foreground);
    line-height: 1.3;
  }
  .desc {
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .note {
    margin-top: 0.125rem;
    font-size: var(--text-2xs);
    line-height: 1.4;
    color: var(--color-warning, #d4b483);
  }
</style>
