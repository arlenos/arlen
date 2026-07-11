<script lang="ts">
  /// Single-select pill row (action mode, schedule modes, access tier, …).
  /// Canonical replacement for the bespoke `seg-pill` markup re-invented per
  /// page. Bindable `value` + `onchange`, forwards `id` for deep-link/search.
  /// An option with an `icon` renders as a compact glyph pill (toolbar
  /// view switchers); its label becomes the accessible name and tooltip.
  import type { Component } from "svelte";
  import * as Tooltip from "../tooltip";

  /// Any icon component taking size/strokeWidth (a Lucide icon from
  /// whichever package instance the host app carries — the nominal
  /// Lucide types differ across installs, the shape does not).
  type IconLike = Component<{ size?: number | string; strokeWidth?: number | string }>;

  let {
    options,
    value = $bindable(),
    id,
    disabled = false,
    ariaLabel,
    onchange,
    class: className,
  }: {
    /// The selectable options, in display order. With `icon` set the
    /// pill shows the glyph only; `label` stays the accessible name.
    options: { value: string; label: string; icon?: IconLike }[];
    /// The currently selected value (bindable).
    value: string;
    /// Optional anchor id for deep-link scroll-to-setting.
    id?: string;
    disabled?: boolean;
    ariaLabel?: string;
    onchange?: (value: string) => void;
    class?: string;
  } = $props();

  // Button refs for roving-focus keyboard navigation.
  let btns = $state<HTMLButtonElement[]>([]);

  // Roving tabindex: exactly one radio is tabbable — the selected one, or the
  // first when nothing is selected yet (the WAI-ARIA radio-group pattern).
  const tabbableIndex = $derived.by(() => {
    const i = options.findIndex((o) => o.value === value);
    return i >= 0 ? i : 0;
  });

  function select(v: string) {
    if (disabled || v === value) return;
    value = v;
    onchange?.(v);
  }

  /// Arrow/Home/End move focus AND selection (selection follows focus, per the
  /// radio-group model), wrapping at the ends.
  function onKeydown(e: KeyboardEvent, i: number) {
    if (disabled) return;
    let next = i;
    switch (e.key) {
      case "ArrowRight":
      case "ArrowDown":
        next = (i + 1) % options.length;
        break;
      case "ArrowLeft":
      case "ArrowUp":
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
    btns[next]?.focus();
  }
</script>

<div class="seg {className ?? ''}" {id} role="radiogroup" aria-label={ariaLabel}>
  {#each options as opt, i (opt.value)}
    {#if opt.icon}
      {@const Icon = opt.icon}
      <Tooltip.Root>
        <Tooltip.Trigger>
          {#snippet child({ props })}
            <button
              {...props}
              bind:this={btns[i]}
              type="button"
              role="radio"
              aria-checked={value === opt.value}
              tabindex={i === tabbableIndex ? 0 : -1}
              {disabled}
              class="seg-pill icon-pill"
              class:active={value === opt.value}
              aria-label={opt.label}
              onclick={() => select(opt.value)}
              onkeydown={(e) => onKeydown(e, i)}
            >
              <Icon size={15} strokeWidth={1.75} />
            </button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.TooltipContent side="bottom">{opt.label}</Tooltip.TooltipContent>
      </Tooltip.Root>
    {:else}
      <button
        bind:this={btns[i]}
        type="button"
        role="radio"
        aria-checked={value === opt.value}
        tabindex={i === tabbableIndex ? 0 : -1}
        {disabled}
        class="seg-pill"
        class:active={value === opt.value}
        onclick={() => select(opt.value)}
        onkeydown={(e) => onKeydown(e, i)}
      >
        {opt.label}
      </button>
    {/if}
  {/each}
</div>

<style>
  .seg {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-input);
    /* The pills hug the inside; their corners follow this radius minus the
       2px padding (rounding-fix.md). */
    --container-radius: var(--radius-input);
    --container-inset: 2px;
    background: var(--control-bg);
    border: 1px solid var(--control-border);
  }

  .seg-pill {
    appearance: none;
    border: 1px solid transparent;
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    font-size: var(--text-sm);
    font-weight: 500;
    padding: 4px 12px;
    min-height: calc(var(--height-control, 28px) - 6px);
    border-radius: max(0px, calc(var(--container-radius) - var(--container-inset)));
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out),
      border-color var(--duration-fast) var(--ease-out);
  }

  .seg-pill.icon-pill {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    width: calc(var(--height-control, 28px) + 2px);
  }

  .seg-pill:hover:not(:disabled):not(.active) {
    background: var(--control-bg-hover);
    color: var(--foreground);
  }

  .seg-pill.active {
    background: color-mix(in srgb, var(--foreground) 15%, transparent);
    border-color: color-mix(in srgb, var(--foreground) 30%, transparent);
    color: var(--foreground);
  }

  .seg:has(.seg-pill:disabled) {
    opacity: var(--control-disabled-opacity, 0.5);
  }
</style>
