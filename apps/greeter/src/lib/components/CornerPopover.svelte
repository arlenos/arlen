<script lang="ts">
  /// A quiet corner control: an icon trigger that opens a small menu above
  /// it. Shared by the power and accessibility entries so both behave and
  /// look identical (click outside or Escape closes). The menu opens upward
  /// since these live on the bottom bar.
  import type { Component, Snippet } from "svelte";

  let {
    icon: Icon,
    label,
    align = "left",
    id,
    children,
  }: {
    icon: Component<{ size?: number; strokeWidth?: number }>;
    label: string;
    align?: "left" | "right";
    id?: string;
    children: Snippet<[() => void]>;
  } = $props();

  let open = $state(false);
  let rootEl = $state<HTMLDivElement | null>(null);

  function close() {
    open = false;
  }

  function onwindowpointerdown(e: PointerEvent) {
    if (open && rootEl && !rootEl.contains(e.target as Node)) close();
  }
  function onwindowkeydown(e: KeyboardEvent) {
    if (open && e.key === "Escape") close();
  }
</script>

<svelte:window onpointerdown={onwindowpointerdown} onkeydown={onwindowkeydown} />

<div class="corner" class:align-right={align === "right"} bind:this={rootEl}>
  {#if open}
    <div class="menu" role="menu">
      {@render children(close)}
    </div>
  {/if}
  <button
    type="button"
    class="trigger"
    {id}
    aria-label={label}
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <Icon size={20} strokeWidth={1.75} />
  </button>
</div>

<style>
  .corner {
    position: relative;
    display: inline-flex;
  }
  /* Topbar-register icon button: quiet, flat, the input radius (matches the
     shell status glyphs and UserRowTile icons). */
  .trigger {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: calc(2rem * var(--greeter-scale, 1));
    height: calc(2rem * var(--greeter-scale, 1));
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    transition:
      background-color var(--duration-fast) var(--ease-out),
      color var(--duration-fast) var(--ease-out);
  }
  .trigger:hover,
  .trigger[aria-expanded="true"] {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    color: var(--foreground);
  }
  /* Flat flyout, the shell UserRowTile power-menu recipe: opaque shell
     surface, 1px hairline, input radius, a quiet float shadow. No blur. */
  .menu {
    position: absolute;
    bottom: calc(100% + 0.5rem);
    left: 0;
    min-width: 16rem;
    padding: 0.25rem;
    border-radius: var(--radius-input);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--foreground) 12%, transparent);
    box-shadow: var(--shadow-md);
    animation: greeter-menu-in var(--duration-medium) var(--ease-out);
  }
  .align-right .menu {
    left: auto;
    right: 0;
  }
  :global([data-contrast="high"]) .menu {
    background: #000000;
    border-color: #ffffff;
  }
  @keyframes greeter-menu-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
</style>
