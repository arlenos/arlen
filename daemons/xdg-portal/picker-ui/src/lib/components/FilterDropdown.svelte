<script lang="ts">
  /// The caller's file-type filter chooser. Portal-specific (the open/
  /// save caller supplies the patterns), skinned on the kit tokens. A
  /// controlled view of the picker UI store's `activeFilter`; the menu
  /// opens upward because it lives in the footer. Filter init lives in
  /// `+page.svelte` so it can honour the request's `currentFilter`.
  import { ChevronDown } from "@lucide/svelte";

  import type { FileFilter } from "$lib/types/protocol";
  import { getUiState, setActiveFilter } from "$lib/stores/pickerUi.svelte";

  let { filters }: { filters: FileFilter[] } = $props();
  const ui = getUiState();

  let open = $state(false);

  function pick(filter: FileFilter | null) {
    setActiveFilter(filter);
    open = false;
  }

  let label = $derived(ui.activeFilter?.name ?? "All files");
</script>

{#if filters.length > 0}
  <div class="filter">
    {#if open}
      <button
        type="button"
        class="scrim"
        aria-label="Close filter menu"
        onclick={() => (open = false)}
      ></button>
    {/if}
    <button type="button" class="trigger" onclick={() => (open = !open)}>
      <span>{label}</span>
      <ChevronDown class="size-3" strokeWidth={2} />
    </button>
    {#if open}
      <ul class="menu" role="listbox">
        {#each filters as filter (filter.name)}
          <li>
            <button
              type="button"
              class="item"
              class:active={ui.activeFilter?.name === filter.name}
              onclick={() => pick(filter)}
            >
              {filter.name}
            </button>
          </li>
        {/each}
        <li class="separator" aria-hidden="true"></li>
        <li>
          <button
            type="button"
            class="item"
            class:active={!ui.activeFilter}
            onclick={() => pick(null)}
          >
            All files
          </button>
        </li>
      </ul>
    {/if}
  </div>
{/if}

<style>
  .filter {
    position: relative;
  }

  /* A full-window scrim closes the menu on any outside click without a
     document listener (the picker has no global click router). */
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 4;
    background: transparent;
    border: none;
    padding: 0;
  }

  .trigger {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: var(--height-control);
    padding: 0 10px;
    background: transparent;
    color: var(--color-fg-app);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-button);
    font-size: 0.8125rem;
    transition: background-color var(--duration-fast) var(--ease-out);
  }

  .trigger:hover {
    background: color-mix(in srgb, var(--color-fg-app) 6%, transparent);
  }

  .menu {
    position: absolute;
    bottom: calc(100% + 4px);
    left: 0;
    z-index: 5;
    min-width: 184px;
    margin: 0;
    padding: 4px;
    list-style: none;
    background: var(--color-bg-card);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-lg, 0 12px 32px rgba(0, 0, 0, 0.4));
  }

  .item {
    display: block;
    width: 100%;
    padding: 6px 10px;
    background: transparent;
    color: var(--color-fg-app);
    border: none;
    border-radius: var(--radius-button);
    text-align: left;
    font-size: 0.8125rem;
  }

  .item:hover {
    background: color-mix(in srgb, var(--color-fg-app) 8%, transparent);
  }

  .item.active {
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    color: var(--color-fg-app);
  }

  .separator {
    height: 1px;
    margin: 4px 0;
    background: var(--color-border);
  }
</style>
