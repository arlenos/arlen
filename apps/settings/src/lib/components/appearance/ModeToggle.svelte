<script lang="ts">
  /// Compact segmented control for theme mode. Fits in a Row.
  import { Sun, Moon } from "lucide-svelte";
  import type { ThemeMode } from "$lib/stores/theme";

  let {
    value,
    onchange,
  }: {
    value: ThemeMode;
    onchange: (mode: ThemeMode) => void;
  } = $props();

  interface Option {
    id: ThemeMode;
    label: string;
    icon: typeof Sun;
  }

  const OPTIONS: Option[] = [
    { id: "light", label: "Light", icon: Sun },
    { id: "dark", label: "Dark", icon: Moon },
  ];

  const activeIndex = $derived(
    Math.max(0, OPTIONS.findIndex((o) => o.id === value))
  );
</script>

<div
  class="segmented"
  role="radiogroup"
  aria-label="Theme mode"
  style="--active-index: {activeIndex};"
>
  <div class="indicator"></div>
  {#each OPTIONS as opt}
    {@const Icon = opt.icon}
    <button
      type="button"
      role="radio"
      aria-checked={value === opt.id}
      class="segment"
      class:active={value === opt.id}
      onclick={() => onchange(opt.id)}
    >
      <Icon size={12} strokeWidth={2} />
      <span>{opt.label}</span>
    </button>
  {/each}
</div>

<style>
  .segmented {
    position: relative;
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    height: 26px;
    padding: 2px;
    width: 200px;
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 9%, transparent);
    border-radius: var(--radius-md);
  }

  .indicator {
    position: absolute;
    top: 2px;
    bottom: 2px;
    width: calc((100% - 4px) / 2);
    left: calc(2px + (100% - 4px) / 2 * var(--active-index));
    background: color-mix(in srgb, var(--foreground) 11%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 14%, transparent);
    border-radius: var(--radius-md);
    box-shadow:
      0 1px 0 color-mix(in srgb, var(--foreground) 6%, transparent) inset,
      0 1px 2px rgba(0, 0, 0, 0.25);
    transition: left 220ms cubic-bezier(0.4, 0, 0.2, 1);
  }

  .segment {
    position: relative;
    z-index: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    font-size: 0.6875rem;
    font-weight: 500;
    font-family: inherit;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    background: transparent;
    border: none;
    cursor: pointer;
    transition: color 150ms ease;
    -webkit-user-select: none;
    user-select: none;
  }

  .segment:hover {
    color: color-mix(in srgb, var(--foreground) 80%, transparent);
  }
  .segment.active {
    color: var(--foreground);
  }
</style>
