<script lang="ts">
  /// Compact font picker with a custom popover dropdown. No native
  /// <select>. Matches the shell's popover styling: rounded card,
  /// divided items, hover highlight, check icon on the active option.
  import { ChevronDown, Check } from "lucide-svelte";

  interface FontOption {
    value: string;
    label: string;
  }

  let {
    value,
    options,
    ariaLabel,
    onchange,
  }: {
    value: string;
    options: FontOption[];
    ariaLabel?: string;
    onchange: (value: string) => void;
  } = $props();

  let open = $state(false);
  let triggerRef = $state<HTMLButtonElement | null>(null);
  let menuRef = $state<HTMLDivElement | null>(null);

  const current = $derived(
    options.find((o) => o.value === value) ?? options[0]
  );

  function toggle(): void {
    open = !open;
  }

  function select(v: string): void {
    onchange(v);
    open = false;
  }

  // Close on outside click or Escape.
  $effect(() => {
    if (!open) return;

    function onClick(e: MouseEvent) {
      const target = e.target as Node;
      if (
        !triggerRef?.contains(target) &&
        !menuRef?.contains(target)
      ) {
        open = false;
      }
    }

    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        open = false;
        triggerRef?.focus();
      }
    }

    // Wait one frame so the click that opened the menu isn't caught.
    const raf = requestAnimationFrame(() => {
      document.addEventListener("click", onClick);
      document.addEventListener("keydown", onKey);
    });

    return () => {
      cancelAnimationFrame(raf);
      document.removeEventListener("click", onClick);
      document.removeEventListener("keydown", onKey);
    };
  });
</script>

<div class="wrap">
  <button
    bind:this={triggerRef}
    type="button"
    class="trigger"
    class:open
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label={ariaLabel}
    onclick={toggle}
  >
    <span
      class="trigger-label"
      style="font-family: '{current.value}', ui-sans-serif, monospace;"
    >
      {current.label}
    </span>
    <ChevronDown size={12} strokeWidth={2} class="trigger-chev" />
  </button>

  {#if open}
    <div bind:this={menuRef} class="menu" role="listbox" aria-label={ariaLabel}>
      {#each options as opt}
        {@const selected = opt.value === value}
        <button
          type="button"
          role="option"
          aria-selected={selected}
          class="item"
          class:selected
          onclick={() => select(opt.value)}
        >
          <span
            class="item-label"
            style="font-family: '{opt.value}', ui-sans-serif, monospace;"
          >
            {opt.label}
          </span>
          {#if selected}
            <Check size={12} strokeWidth={2.5} class="item-check" />
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
    width: 200px;
  }

  /* ── Trigger ──────────────────────────────────────── */
  .trigger {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    height: 28px;
    padding: 0 0.625rem 0 0.75rem;
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

  .trigger:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    border-color: color-mix(in srgb, var(--foreground) 15%, transparent);
  }

  .trigger.open {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    border-color: color-mix(in srgb, var(--foreground) 20%, transparent);
  }

  .trigger-label {
    flex: 1;
    min-width: 0;
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--foreground);
    line-height: 1;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  :global(.trigger-chev) {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    flex-shrink: 0;
    transition: transform 150ms ease;
  }

  .trigger.open :global(.trigger-chev) {
    transform: rotate(180deg);
  }

  /* ── Menu ─────────────────────────────────────────── */
  .menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    z-index: 50;
    padding: 4px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--background) 94%, var(--foreground) 8%);
    border: 1px solid
      color-mix(in srgb, var(--foreground) 15%, transparent);
    box-shadow:
      0 10px 30px -10px rgba(0, 0, 0, 0.6),
      0 4px 12px -4px rgba(0, 0, 0, 0.4);
    animation: menu-in 120ms cubic-bezier(0.4, 0, 0.2, 1);
  }

  @keyframes menu-in {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .item {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    width: 100%;
    min-height: 28px;
    padding: 0 0.625rem 0 0.75rem;
    border: none;
    background: transparent;
    border-radius: var(--radius-md);
    cursor: pointer;
    font-family: inherit;
    transition: background-color 100ms ease;
  }

  .item:hover {
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
  }

  .item.selected {
    background: color-mix(in srgb, var(--foreground) 6%, transparent);
  }

  .item-label {
    flex: 1;
    min-width: 0;
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--foreground);
    line-height: 1;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  :global(.item-check) {
    color: var(--color-accent);
    flex-shrink: 0;
  }
</style>
