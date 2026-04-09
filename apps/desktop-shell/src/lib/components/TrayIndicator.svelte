<script lang="ts">
  /// System tray indicator: shows a chevron when SNI items are registered.

  import { togglePopover, hoverPopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { ChevronDown } from "lucide-svelte";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";

  interface SniItem {
    service: string;
    id: string;
    category: string;
    status: string;
    title: string;
    icon_name: string;
    icon_pixmap: string | null;
    tooltip_title: string | null;
    tooltip_description: string | null;
    menu_path: string | null;
  }

  let items = $state<SniItem[]>([]);
  let hasAttention = $state(false);

  async function loadItems() {
    try {
      items = await invoke<SniItem[]>("get_sni_items");
      hasAttention = items.some((i) => i.status === "NeedsAttention");
    } catch {}
  }

  onMount(() => {
    loadItems();
    const unlisten = listen("sni-items-changed", () => loadItems());
    return () => {
      unlisten.then((fn) => fn());
    };
  });

  const visible = $derived(items.length > 0);
</script>

{#if visible}
  <Tooltip.Root>
    <Tooltip.Trigger>
      {#snippet child({ props })}
        <button
          {...props}
          class="tray-btn"
          class:has-attention={hasAttention}
          onclick={() => togglePopover("tray")}
          onmouseenter={() => hoverPopover("tray")}
        >
          <ChevronDown size={14} strokeWidth={1.5} />
          {#if hasAttention}
            <span class="attention-dot"></span>
          {/if}
        </button>
      {/snippet}
    </Tooltip.Trigger>
    <Tooltip.Content>
      <p>Background Apps ({items.length})</p>
    </Tooltip.Content>
  </Tooltip.Root>
{/if}

<style>
  .tray-btn {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .tray-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }
  .attention-dot {
    position: absolute;
    top: 4px;
    right: 4px;
    width: 6px;
    height: 6px;
    background: var(--color-error);
    border-radius: 50%;
  }
</style>
