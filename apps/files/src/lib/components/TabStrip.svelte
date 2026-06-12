<script lang="ts">
  /// The tabs, living in the headerbar (two-level chrome): visible
  /// only with more than one tab open. A tab shows its folder name;
  /// the close affordance appears on hover, Ctrl+W closes the active
  /// one. The gaps around the chips stay window-drag area.
  import { X } from "lucide-svelte";
  import { get } from "svelte/store";
  import { activeTabId, closeTab, selectTab, tabs, type Tab } from "$lib/stores/tabs";

  const tabLabel = (tab: Tab): string => {
    const p = get(tab.controller.path);
    const name = p.split("/").filter(Boolean).pop();
    return name ?? "/";
  };

  // Paths change as tabs navigate; the labels record is rebuilt from
  // a local accumulator so the effect never reads what it writes.
  let labels = $state<Record<number, string>>({});
  $effect(() => {
    const acc: Record<number, string> = {};
    const unsubs = $tabs.map((t) =>
      t.controller.path.subscribe((p) => {
        acc[t.id] = p.split("/").filter(Boolean).pop() ?? "/";
        labels = { ...acc };
      }),
    );
    return () => unsubs.forEach((u) => u());
  });
</script>

{#if $tabs.length > 1}
  <div class="tab-strip" role="tablist">
    {#each $tabs as tab (tab.id)}
      <div class="ts-tab" class:active={tab.id === $activeTabId}>
        <button
          class="ts-label"
          role="tab"
          aria-selected={tab.id === $activeTabId}
          onclick={() => selectTab(tab.id)}
        >
          {labels[tab.id] ?? tabLabel(tab)}
        </button>
        <button
          class="ts-close"
          aria-label="Close tab"
          onclick={() => closeTab(tab.id)}
        >
          <X size={12} strokeWidth={2} />
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .tab-strip {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    overflow: hidden;
  }

  .ts-tab {
    display: inline-flex;
    align-items: center;
    height: var(--height-control, 28px);
    border-radius: var(--radius-input);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .ts-tab:hover {
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .ts-tab.active {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }

  .ts-label {
    height: 100%;
    max-width: 12rem;
    padding: 0 4px 0 10px;
    border: none;
    background: transparent;
    color: inherit;
    font-size: 0.75rem;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ts-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.25rem;
    height: 1.25rem;
    margin-right: 4px;
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    color: inherit;
    opacity: 0;
    transition: opacity var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .ts-tab:hover .ts-close,
  .ts-tab.active .ts-close {
    opacity: 1;
  }
  .ts-close:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
</style>
