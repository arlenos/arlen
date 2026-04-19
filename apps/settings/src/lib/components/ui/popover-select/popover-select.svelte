<script lang="ts" module>
  export interface PopoverSelectOption {
    value: string;
    label: string;
  }
</script>

<script lang="ts">
  /// Generic popover-based dropdown.
  ///
  /// Canonical source lives in `sdk/ui-kit`; consuming apps
  /// (`app-settings`, `desktop-shell`) keep file copies under their
  /// own `src/lib/components/ui/` because Tauri's bundler does not
  /// cope with symlinked Svelte components (Tailwind scope hashing
  /// breaks). Sync by copying when this file changes.
  ///
  /// A trigger button collapsed to a single line shows the current
  /// selection; clicking opens a card-style listbox below. Escape /
  /// outside-click dismisses it, and the active option carries a
  /// trailing check icon. The menu is rendered into `document.body`
  /// via an inline portal action so `overflow: hidden` ancestors
  /// (SettingsGroup cards, shell popovers) cannot clip it.
  ///
  /// Pass `renderLabel` to customise how each option renders. Default
  /// is plain text. The Typography font picker uses it to render each
  /// option in its own font family.

  import { ChevronDown, Check } from "lucide-svelte";
  import type { Snippet } from "svelte";

  type Props = {
    value: string;
    options: PopoverSelectOption[];
    ariaLabel?: string;
    /// CSS width for the trigger + menu. Passed through as `width`
    /// so callers can say `"200px"` (FontSelect) or `"100%"`
    /// (full-width settings row). Defaults to `"200px"` to match
    /// Typography.
    width?: string;
    /// Shown in the trigger when `options` is empty or none matches
    /// `value`. The default covers the common "still loading" case
    /// for callers that populate `options` asynchronously (AudioPopover
    /// polls the devices after mount — without a placeholder the
    /// trigger would try to render `undefined.label` and crash the
    /// whole surrounding popover).
    placeholder?: string;
    onchange: (value: string) => void;
    /// Optional per-option renderer. Receives the option and whether
    /// it's the currently-selected one. When omitted, the component
    /// renders `option.label` as plain text.
    renderLabel?: Snippet<[PopoverSelectOption, boolean]>;
  };

  let {
    value,
    options,
    ariaLabel,
    width = "200px",
    placeholder = "None",
    onchange,
    renderLabel,
  }: Props = $props();

  let open = $state(false);
  let triggerRef = $state<HTMLButtonElement | null>(null);
  let menuRef = $state<HTMLDivElement | null>(null);

  /// Menu geometry relative to the viewport. `position: fixed` + live
  /// updates on scroll/resize lets the menu escape ancestors that
  /// clip overflow (SettingsGroup uses `overflow: hidden` to round its
  /// corners, which was cutting the dropdown off).
  let menuPos = $state({ top: 0, left: 0, width: 0 });

  /// `undefined` means either `options` is empty or no entry matches
  /// `value`. Callers that provide a non-empty catalogue will always
  /// get a defined value, but async-populated options (e.g. device
  /// lists) have a transient empty phase — the template guards for it.
  const current = $derived<PopoverSelectOption | undefined>(
    options.find((o) => o.value === value) ?? options[0]
  );

  function toggle(): void {
    open = !open;
  }

  function select(v: string): void {
    onchange(v);
    open = false;
  }

  function updatePosition(): void {
    if (!triggerRef) return;
    const r = triggerRef.getBoundingClientRect();
    menuPos = {
      top: r.bottom + 4,
      left: r.left,
      width: r.width,
    };
  }

  /// Move the menu into `document.body` so it is not clipped by any
  /// overflow-hidden ancestor. Svelte still owns the node — `bind:this`
  /// on the same element keeps `menuRef` valid across the move, so the
  /// outside-click check below works unchanged.
  function portal(node: HTMLElement) {
    document.body.appendChild(node);
    return {
      destroy() {
        if (node.parentNode === document.body) {
          node.remove();
        }
      },
    };
  }

  // Close on outside click or Escape. Wrapped in one `requestAnimationFrame`
  // tick so the click that opened the menu doesn't immediately close it.
  // Scroll + resize listeners keep the portal-ed menu pinned to the
  // trigger as the user scrolls a parent container.
  $effect(() => {
    if (!open) return;

    updatePosition();

    function onClick(e: MouseEvent) {
      const target = e.target as Node;
      if (!triggerRef?.contains(target) && !menuRef?.contains(target)) {
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

    function onScroll() {
      updatePosition();
    }
    function onResize() {
      updatePosition();
    }

    const raf = requestAnimationFrame(() => {
      document.addEventListener("click", onClick);
      document.addEventListener("keydown", onKey);
    });
    // `capture: true` on scroll catches every scroll container in the
    // ancestry, not just the window — settings panels live inside a
    // scrollable content column.
    window.addEventListener("scroll", onScroll, true);
    window.addEventListener("resize", onResize);

    return () => {
      cancelAnimationFrame(raf);
      document.removeEventListener("click", onClick);
      document.removeEventListener("keydown", onKey);
      window.removeEventListener("scroll", onScroll, true);
      window.removeEventListener("resize", onResize);
    };
  });
</script>

<div class="wrap" style="width: {width};">
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
    <span class="trigger-label" class:is-placeholder={!current}>
      {#if current && renderLabel}
        {@render renderLabel(current, true)}
      {:else if current}
        {current.label}
      {:else}
        {placeholder}
      {/if}
    </span>
    <ChevronDown size={12} strokeWidth={2} class="trigger-chev" />
  </button>
</div>

{#if open}
  <div
    use:portal
    bind:this={menuRef}
    class="menu"
    role="listbox"
    aria-label={ariaLabel}
    style="top: {menuPos.top}px; left: {menuPos.left}px; width: {menuPos.width}px;"
  >
    {#each options as opt (opt.value)}
      {@const selected = opt.value === value}
      <button
        type="button"
        role="option"
        aria-selected={selected}
        class="item"
        class:selected
        onclick={() => select(opt.value)}
      >
        <span class="item-label">
          {#if renderLabel}
            {@render renderLabel(opt, selected)}
          {:else}
            {opt.label}
          {/if}
        </span>
        {#if selected}
          <Check size={12} strokeWidth={2.5} class="item-check" />
        {/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .wrap {
    position: relative;
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

  .trigger-label.is-placeholder {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    font-weight: 400;
  }

  :global(.trigger-chev) {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    flex-shrink: 0;
    transition: transform 150ms ease;
  }

  .trigger.open :global(.trigger-chev) {
    transform: rotate(180deg);
  }

  /* ── Menu (portal-ed to document.body) ───────────── */
  /*
   * `position: fixed` + JS-driven top/left lets the menu escape
   * overflow-hidden ancestors like SettingsGroup's rounded card. The
   * inline `style=` on the element supplies the pinned coordinates;
   * the rules here are layout-agnostic.
   */
  .menu {
    position: fixed;
    z-index: 9999;
    max-height: 280px;
    overflow-y: auto;
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
