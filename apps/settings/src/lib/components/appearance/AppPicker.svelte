<script lang="ts">
  /// Combobox-style app picker.
  ///
  /// Always shows the daemon-known apps (sourced from the SQLite
  /// history via `notifications_get_known_apps`). The text input
  /// filters that list — typing narrows it down. Free-form names
  /// can be committed via the "Use ‘…’" footer entry or by pressing
  /// Enter when nothing in the list matches.

  import { ChevronDown, Plus, Search } from "lucide-svelte";

  let {
    knownApps,
    excluded = [],
    placeholder = "Add app...",
    onpick,
  }: {
    knownApps: string[];
    excluded?: string[];
    placeholder?: string;
    onpick: (appName: string) => void;
  } = $props();

  let query = $state("");
  let inputRef = $state<HTMLInputElement | null>(null);
  let menuRef = $state<HTMLDivElement | null>(null);
  let triggerRef = $state<HTMLDivElement | null>(null);
  let open = $state(false);

  const available = $derived(knownApps.filter((a) => !excluded.includes(a)));

  const filtered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    if (!q) return available;
    return available.filter((a) => a.toLowerCase().includes(q));
  });

  const trimmed = $derived(query.trim());

  // Show the "use custom name" footer only when the typed value is
  // not already in the (case-insensitive) known list.
  const showCustomFooter = $derived.by(() => {
    if (!trimmed) return false;
    const lower = trimmed.toLowerCase();
    return !available.some((a) => a.toLowerCase() === lower);
  });

  function pick(name: string) {
    onpick(name);
    query = "";
    open = false;
  }

  function commitFreeform() {
    if (!trimmed) return;
    pick(trimmed);
  }

  function toggleOpen() {
    open = !open;
    if (open) {
      requestAnimationFrame(() => inputRef?.focus());
    }
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      if (filtered.length === 1) {
        pick(filtered[0]);
      } else if (filtered.length > 0 && !trimmed) {
        // Empty query + Enter: pick the first known app.
        pick(filtered[0]);
      } else {
        commitFreeform();
      }
    } else if (e.key === "Escape") {
      open = false;
      inputRef?.blur();
    } else if (e.key === "ArrowDown" && !open) {
      open = true;
    }
  }

  $effect(() => {
    if (!open) return;
    function handleClick(e: MouseEvent) {
      const target = e.target as Node;
      if (!triggerRef?.contains(target) && !menuRef?.contains(target)) {
        open = false;
      }
    }
    const raf = requestAnimationFrame(() => {
      document.addEventListener("click", handleClick);
    });
    return () => {
      cancelAnimationFrame(raf);
      document.removeEventListener("click", handleClick);
    };
  });
</script>

<div class="picker">
  <div bind:this={triggerRef} class="input-wrap" class:open>
    <Search size={11} strokeWidth={2.25} class="picker-search" />
    <input
      bind:this={inputRef}
      type="text"
      class="input"
      value={query}
      {placeholder}
      oninput={(e) => (query = (e.currentTarget as HTMLInputElement).value)}
      onfocus={() => (open = true)}
      onkeydown={onKey}
    />
    <button
      type="button"
      class="chev"
      class:rotated={open}
      aria-label={open ? "Close" : "Open"}
      onclick={toggleOpen}
    >
      <ChevronDown size={12} strokeWidth={2.25} />
    </button>
  </div>

  {#if open}
    <div bind:this={menuRef} class="menu" role="listbox">
      {#if filtered.length === 0 && !trimmed}
        <div class="empty">
          {#if knownApps.length === 0}
            No apps have sent notifications yet. Send one with
            <code>notify-send</code> or type a name below.
          {:else}
            Every known app is already listed above. Type a custom name to
            add it manually.
          {/if}
        </div>
      {:else if filtered.length === 0}
        <div class="empty">
          No matches for "<strong>{trimmed}</strong>".
        </div>
      {:else}
        {#each filtered as app}
          <button
            type="button"
            role="option"
            aria-selected="false"
            class="item"
            onclick={() => pick(app)}
          >
            {app}
          </button>
        {/each}
      {/if}

      {#if showCustomFooter}
        <div class="separator" role="separator"></div>
        <button
          type="button"
          class="item custom"
          onclick={commitFreeform}
          title="Add this name as a custom entry"
        >
          <Plus size={12} strokeWidth={2.5} class="custom-icon" />
          <span>Use "<strong>{trimmed}</strong>"</span>
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .picker {
    position: relative;
    width: 100%;
  }

  .input-wrap {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 0.25rem 0 0.5rem;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 10%, transparent);
    transition:
      background-color 150ms ease,
      border-color 150ms ease;
  }
  .input-wrap:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .input-wrap.open {
    border-color: color-mix(in srgb, var(--color-accent) 40%, transparent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-accent) 18%, transparent);
  }

  :global(.picker-search) {
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    flex-shrink: 0;
  }

  .input {
    flex: 1;
    min-width: 0;
    background: transparent;
    border: none;
    outline: none;
    color: var(--foreground);
    font: inherit;
    font-size: 0.75rem;
    padding: 0;
  }
  .input::placeholder {
    color: color-mix(in srgb, var(--foreground) 38%, transparent);
  }

  .chev {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
    cursor: pointer;
    transition:
      transform 180ms cubic-bezier(0.4, 0, 0.2, 1),
      color 120ms ease,
      background-color 120ms ease;
  }
  .chev:hover {
    color: var(--foreground);
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
  }
  .chev.rotated {
    transform: rotate(180deg);
  }

  .menu {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    z-index: 60;
    padding: 4px;
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--background) 94%, var(--foreground) 8%);
    border: 1px solid color-mix(in srgb, var(--foreground) 15%, transparent);
    box-shadow:
      0 10px 30px -10px rgba(0, 0, 0, 0.6),
      0 4px 12px -4px rgba(0, 0, 0, 0.4);
    max-height: 240px;
    overflow-y: auto;
  }

  .item {
    display: flex;
    width: 100%;
    align-items: center;
    gap: 6px;
    padding: 0.4rem 0.6rem;
    background: transparent;
    border: none;
    border-radius: var(--radius-sm);
    color: var(--foreground);
    font-size: 0.75rem;
    text-align: left;
    cursor: pointer;
    transition: background-color 100ms ease;
  }
  .item:hover {
    background: color-mix(in srgb, var(--foreground) 9%, transparent);
  }
  .item.custom {
    color: var(--color-accent);
    font-weight: 500;
  }
  .item.custom strong {
    color: var(--foreground);
    font-weight: 600;
  }
  :global(.custom-icon) {
    color: var(--color-accent);
    flex-shrink: 0;
  }

  .empty {
    padding: 0.6rem 0.7rem;
    font-size: 0.6875rem;
    line-height: 1.4;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .empty code {
    font-family: var(--font-mono);
    font-size: 0.6875rem;
    padding: 0.05rem 0.3rem;
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
  .empty strong {
    color: var(--foreground);
    font-weight: 600;
  }

  .separator {
    height: 1px;
    margin: 4px 2px;
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
</style>
