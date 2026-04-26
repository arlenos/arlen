<script lang="ts">
  /// Compact accent color picker. Nine swatches in a single row
  /// (8 presets + custom). The active preset is clearly marked with
  /// an outer ring and an inset check icon.
  import { Check, Pipette, Contrast } from "lucide-svelte";
  import { ACCENT_PRESETS, MONO_SENTINEL } from "$lib/stores/theme";
  import { ColorPicker } from "$lib/components/ui/color-picker";

  let {
    value,
    rawOverride,
    onchange,
  }: {
    /// Effective accent hex (already resolved, for rendering the swatch fill).
    value: string;
    /// Raw override string from the config, used to detect the monochrome
    /// sentinel. Undefined when no override is set.
    rawOverride?: string;
    onchange: (value: string) => void;
  } = $props();

  let pickerOpen = $state(false);
  let pickerWrap = $state<HTMLDivElement | null>(null);

  const isMono = $derived(rawOverride === MONO_SENTINEL);
  const activePreset = $derived(
    isMono
      ? undefined
      : ACCENT_PRESETS.find(
          (p) => p.value.toLowerCase() === value.toLowerCase()
        )
  );
  const isCustom = $derived(!isMono && !activePreset);

  function openPicker(): void {
    pickerOpen = !pickerOpen;
  }

  // Close on outside click. Mounted only while the popover is open
  // so the listener cost stays scoped to the user actually interacting.
  $effect(() => {
    if (!pickerOpen) return;
    const handler = (e: MouseEvent) => {
      if (!pickerWrap) return;
      if (!pickerWrap.contains(e.target as Node)) pickerOpen = false;
    };
    // Defer one tick so the click that opened the popover does not
    // immediately close it.
    const id = window.setTimeout(
      () => document.addEventListener("mousedown", handler),
      0,
    );
    return () => {
      window.clearTimeout(id);
      document.removeEventListener("mousedown", handler);
    };
  });
</script>

<div class="swatches">
  {#each ACCENT_PRESETS as preset}
    {@const selected =
      !isMono && value.toLowerCase() === preset.value.toLowerCase()}
    <button
      type="button"
      class="swatch"
      class:selected
      aria-label={preset.name}
      aria-pressed={selected}
      title={preset.name}
      style="background-color: {preset.value};"
      onclick={() => onchange(preset.value)}
    >
      {#if selected}
        <Check size={12} strokeWidth={3} class="swatch-check" />
      {/if}
    </button>
  {/each}

  <button
    type="button"
    class="swatch swatch-mono"
    class:selected={isMono}
    aria-label="Monochrome"
    aria-pressed={isMono}
    title="Monochrome"
    onclick={() => onchange(MONO_SENTINEL)}
  >
    {#if isMono}
      <Check size={12} strokeWidth={3} class="swatch-check-mono" />
    {:else}
      <Contrast size={12} strokeWidth={2.25} class="swatch-mono-icon" />
    {/if}
  </button>

  <div class="swatch-custom-wrap" bind:this={pickerWrap}>
    <button
      type="button"
      class="swatch swatch-custom"
      class:selected={isCustom}
      aria-label="Custom color"
      aria-pressed={isCustom}
      aria-expanded={pickerOpen}
      title="Custom color"
      style={isCustom ? `background-color: ${value};` : undefined}
      onclick={openPicker}
    >
      {#if isCustom}
        <Check size={12} strokeWidth={3} class="swatch-check" />
      {:else}
        <Pipette size={11} strokeWidth={2.5} class="swatch-pipette" />
      {/if}
    </button>

    {#if pickerOpen}
      <div class="swatch-popover">
        <ColorPicker
          {value}
          onchange={(hex) => onchange(hex)}
        />
      </div>
    {/if}
  </div>
</div>

<style>
  .swatches {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .swatch {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-md);
    border: none;
    padding: 0;
    cursor: pointer;
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.35) inset,
      0 1px 2px rgba(0, 0, 0, 0.4);
    transition:
      transform 150ms cubic-bezier(0.4, 0, 0.2, 1),
      box-shadow 150ms ease;
  }

  .swatch:hover:not(.selected) {
    transform: scale(1.1);
  }

  .swatch.selected {
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.35) inset,
      0 0 0 2px var(--background),
      0 0 0 3.5px var(--foreground),
      0 2px 6px rgba(0, 0, 0, 0.5);
  }

  .swatch-mono {
    background: linear-gradient(
      135deg,
      var(--color-fg-primary) 0% 50%,
      var(--color-bg-app) 50% 100%
    );
  }

  :global(.swatch-mono-icon) {
    color: var(--color-fg-primary);
    mix-blend-mode: difference;
    filter: drop-shadow(0 1px 1px rgba(0, 0, 0, 0.3));
  }

  :global(.swatch-check-mono) {
    color: var(--color-fg-primary);
    mix-blend-mode: difference;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.6));
  }

  .swatch-custom:not(.selected) {
    background: conic-gradient(
      from 180deg,
      #6366f1,
      #3b82f6,
      #06b6d4,
      #10b981,
      #f59e0b,
      #ef4444,
      #ec4899,
      #a855f7,
      #6366f1
    );
  }

  :global(.swatch-check) {
    color: #ffffff;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.6));
  }

  :global(.swatch-pipette) {
    color: #ffffff;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.6));
  }

  .swatch-custom-wrap {
    position: relative;
    display: inline-flex;
  }

  .swatch-popover {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 50;
  }
</style>
