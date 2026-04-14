<script lang="ts">
  /// Compact color picker for window border colours. Offers a sentinel
  /// pill ("Use Accent" / "Use Subtle") plus a custom color chip. The
  /// sentinel is stored as `$accent` / `$border` so the compositor can
  /// re-resolve it on theme mode switches.

  import { Check, Pipette } from "lucide-svelte";

  let {
    value,
    sentinel,
    sentinelLabel,
    sentinelSwatch,
    onchange,
  }: {
    /// Current raw value from the config (hex or sentinel), may be undefined.
    value: string | undefined;
    /// Sentinel string this picker treats as "auto/linked" (e.g. `$accent`).
    sentinel: string;
    /// User-facing label for the sentinel pill.
    sentinelLabel: string;
    /// CSS colour used to render the sentinel swatch fill.
    sentinelSwatch: string;
    onchange: (value: string) => void;
  } = $props();

  let customInput = $state<HTMLInputElement | null>(null);

  const isSentinel = $derived(value === sentinel);
  const isCustom = $derived(!isSentinel && typeof value === "string");

  function openPicker() {
    customInput?.click();
  }

  function onPickerInput(e: Event) {
    onchange((e.currentTarget as HTMLInputElement).value);
  }
</script>

<div class="bpick">
  <button
    type="button"
    class="pill"
    class:selected={isSentinel}
    aria-pressed={isSentinel}
    onclick={() => onchange(sentinel)}
  >
    <span class="pill-dot" style="background: {sentinelSwatch};"></span>
    <span class="pill-label">{sentinelLabel}</span>
    {#if isSentinel}
      <Check size={12} strokeWidth={2.5} class="pill-check" />
    {/if}
  </button>

  <button
    type="button"
    class="chip"
    class:selected={isCustom}
    aria-label="Custom color"
    aria-pressed={isCustom}
    title="Custom color"
    style={isCustom ? `background-color: ${value};` : undefined}
    onclick={openPicker}
  >
    {#if isCustom}
      <Check size={12} strokeWidth={3} class="chip-check" />
    {:else}
      <Pipette size={11} strokeWidth={2.5} class="chip-pipette" />
    {/if}
  </button>

  <input
    bind:this={customInput}
    type="color"
    value={isCustom ? value : "#000000"}
    oninput={onPickerInput}
    class="sr-only"
    tabindex={-1}
    aria-hidden="true"
  />
</div>

<style>
  .bpick {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 0.5rem 0 0.4rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 10%, transparent);
    cursor: pointer;
    font-family: inherit;
    transition:
      background-color 150ms ease,
      border-color 150ms ease;
  }

  .pill:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }

  .pill.selected {
    background: color-mix(in srgb, var(--color-accent) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-accent) 30%, transparent);
  }

  .pill-dot {
    width: 12px;
    height: 12px;
    border-radius: 999px;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.35) inset;
  }

  .pill-label {
    font-size: 0.6875rem;
    font-weight: 500;
    color: var(--foreground);
    line-height: 1;
    -webkit-user-select: none;
    user-select: none;
  }

  :global(.pill-check) {
    color: var(--color-accent);
  }

  .chip {
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
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.35) inset,
      0 1px 2px rgba(0, 0, 0, 0.4);
    transition:
      transform 150ms cubic-bezier(0.4, 0, 0.2, 1),
      box-shadow 150ms ease;
  }

  .chip:hover:not(.selected) {
    transform: scale(1.08);
  }

  .chip.selected {
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.35) inset,
      0 0 0 2px var(--background),
      0 0 0 3.5px var(--foreground),
      0 2px 6px rgba(0, 0, 0, 0.5);
  }

  :global(.chip-check) {
    color: #ffffff;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.6));
  }

  :global(.chip-pipette) {
    color: #ffffff;
    filter: drop-shadow(0 1px 2px rgba(0, 0, 0, 0.6));
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    border: 0;
  }
</style>
