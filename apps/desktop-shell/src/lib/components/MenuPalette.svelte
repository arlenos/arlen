<script lang="ts">
  /// The app-menu context popup (tier-c-gaps-plan.md §2): the active window's menu as
  /// a floating, searchable command-palette, so any menu item is reachable by
  /// keyboard. On the kit command primitive like the terminal history palette; the
  /// data is the real (flattened) active-window menu, a fixture under vite.
  import {
    Command,
    CommandInput,
    CommandList,
    CommandItem,
    CommandEmpty,
    CommandShortcut,
  } from "@arlen/ui-kit/components/ui/command";
  import { Check } from "lucide-svelte";
  import {
    menuPaletteOpen,
    paletteItems,
    activate,
    closeMenuPalette,
  } from "$lib/stores/menuPalette";

  let query = $state("");
  // Reset the query each time the palette opens, so it never shows a stale filter.
  $effect(() => {
    if ($menuPaletteOpen) query = "";
  });

  function onWindowKeydown(e: KeyboardEvent) {
    if ($menuPaletteOpen && e.key === "Escape") {
      e.preventDefault();
      closeMenuPalette();
    }
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

{#if $menuPaletteOpen}
  <div
    class="mp-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) closeMenuPalette();
    }}
  >
    <div class="mp-card" role="dialog" aria-modal="true" aria-label="App menu" tabindex="-1">
      <Command>
        <CommandInput placeholder="Search this app's menu" autofocus bind:value={query} />
        <CommandList class="mp-list">
          <CommandEmpty>No matching menu items.</CommandEmpty>
          {#each $paletteItems as it (it.action + it.label)}
            <CommandItem
              value={`${it.label} ${it.path.join(" ")}`}
              disabled={it.disabled}
              onSelect={() => activate(it)}
            >
              {#if it.checked}
                <Check class="mp-check" size={13} strokeWidth={2.5} />
              {:else}
                <span class="mp-check-spacer" aria-hidden="true"></span>
              {/if}
              <span class="mp-label">{it.label}</span>
              {#if it.path.length > 0}
                <span class="mp-path">{it.path.join(" › ")}</span>
              {/if}
              {#if it.shortcut}
                <CommandShortcut>{it.shortcut}</CommandShortcut>
              {/if}
            </CommandItem>
          {/each}
        </CommandList>
      </Command>
    </div>
  </div>
{/if}

<style>
  .mp-backdrop {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 18vh;
    background: var(--color-bg-overlay, #00000080);
  }
  .mp-card {
    width: min(560px, calc(100vw - 48px));
    border: 1px solid color-mix(in srgb, var(--foreground) 15%, transparent);
    border-radius: var(--radius-card);
    background: var(--color-bg-card);
    box-shadow: var(--shadow-lg);
    overflow: hidden;
  }
  :global(.mp-list) {
    max-height: 340px;
    padding: 4px;
    scrollbar-width: none;
  }
  :global(.mp-check) {
    flex-shrink: 0;
    color: var(--foreground);
  }
  .mp-check-spacer {
    flex-shrink: 0;
    width: 14px;
  }
  .mp-label {
    font-size: 0.8125rem;
    color: var(--foreground);
    white-space: nowrap;
  }
  .mp-path {
    flex: 1;
    min-width: 0;
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 42%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
