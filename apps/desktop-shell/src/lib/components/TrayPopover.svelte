<script lang="ts">
  /// System tray popover: lists registered SNI items with context menu support.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Layers } from "lucide-svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import * as ContextMenu from "$lib/components/ui/context-menu/index.js";
  import SniContextMenuContent from "$lib/components/SniContextMenuContent.svelte";

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
  let loading = $state(false);

  async function loadItems() {
    loading = true;
    try {
      items = await invoke<SniItem[]>("get_sni_items");
    } catch {}
    loading = false;
  }

  async function handleActivate(service: string) {
    try {
      await invoke("activate_sni_item", { service });
    } catch {}
    closePopover();
  }

  $effect(() => {
    if ($activePopover === "tray") {
      loadItems();
    }
  });

  function getInitials(id: string): string {
    return id.substring(0, 2).toUpperCase();
  }
</script>

{#if $activePopover === "tray"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-backdrop" onclick={closePopover}></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-panel pop-tray shell-popover" onclick={(e) => e.stopPropagation()}>
    <PopoverHeader icon={Layers} title="Background Apps" />
    <div class="pop-body">
      {#if loading}
        <div class="tray-empty">Loading...</div>
      {:else if items.length === 0}
        <div class="tray-empty">No background apps</div>
      {:else}
        <div class="tray-list">
          {#each items as item}
            <ContextMenu.Root>
              <ContextMenu.Trigger>
                {#snippet child({ props })}
                  <button
                    {...props}
                    class="tray-item"
                    class:attention={item.status === "NeedsAttention"}
                    onclick={(e) => { e.stopPropagation(); handleActivate(item.service); }}
                    title={item.tooltip_description || item.tooltip_title || item.title}
                  >
                    <div class="tray-item-icon">
                      {#if item.icon_pixmap}
                        <img src={item.icon_pixmap} alt={item.id} />
                      {:else}
                        {getInitials(item.id)}
                      {/if}
                    </div>
                    <div class="tray-item-info">
                      <span class="tray-item-title">{item.title || item.id || "Unknown"}</span>
                      {#if item.tooltip_description && item.tooltip_description !== item.title}
                        <span class="tray-item-subtitle">{item.tooltip_description}</span>
                      {/if}
                    </div>
                    {#if item.status === "NeedsAttention"}
                      <span class="tray-item-badge"></span>
                    {/if}
                  </button>
                {/snippet}
              </ContextMenu.Trigger>
              {#if item.menu_path}
                <SniContextMenuContent service={item.service} menuPath={item.menu_path} />
              {/if}
            </ContextMenu.Root>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .pop-backdrop { position: fixed; inset: 0; z-index: 90; }
  .pop-panel {
    position: fixed; top: 40px; z-index: 100; border-radius: 10px;
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-shell);
    display: flex; flex-direction: column;
    animation: pop-open 100ms ease-out both;
  }
  .pop-tray { right: 140px; width: 260px; }
  .pop-body { padding: 8px; display: flex; flex-direction: column; gap: 2px; }
  @keyframes pop-open { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }

  .tray-empty { padding: 20px; text-align: center; color: color-mix(in srgb, var(--color-fg-shell) 40%, transparent); font-size: 0.75rem; }

  .tray-list { display: flex; flex-direction: column; gap: 2px; max-height: 240px; overflow-y: auto; }

  .tray-item {
    display: flex; align-items: center; gap: 10px;
    padding: 8px 10px; background: transparent; border: none; border-radius: 6px;
    color: var(--color-fg-shell); cursor: pointer; text-align: left; width: 100%;
    transition: background-color 0.1s ease;
  }
  .tray-item:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .tray-item.attention { background: color-mix(in srgb, var(--color-fg-shell) 5%, transparent); }

  .tray-item-icon {
    width: 24px; height: 24px; display: flex; align-items: center; justify-content: center;
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-radius: 4px; font-size: 0.625rem; font-weight: 600; flex-shrink: 0;
  }
  .tray-item-icon img { width: 18px; height: 18px; object-fit: contain; }
  .tray-item-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .tray-item-title { font-size: 0.8125rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; display: block; }
  .tray-item-subtitle { font-size: 0.6875rem; color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; display: block; }
  .tray-item-badge { width: 8px; height: 8px; background: var(--color-error); border-radius: 50%; flex-shrink: 0; }
</style>
