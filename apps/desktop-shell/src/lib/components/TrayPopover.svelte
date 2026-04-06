<script lang="ts">
  /// System tray popover: lists registered SNI items.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Layers, Settings } from "lucide-svelte";

  interface SniItem {
    service: string;
    id: string;
    category: string;
    status: string;
    title: string;
    icon_name: string;
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
    <div class="pop-header">
      <Layers size={16} strokeWidth={1.5} />
      <span class="pop-title">Background Apps</span>
      <button class="pop-settings-btn" onclick={(e) => { e.stopPropagation(); closePopover(); }}>
        <Settings size={14} strokeWidth={1.5} />
      </button>
    </div>
    <div class="pop-body">
      {#if loading}
        <div class="tray-empty">Loading...</div>
      {:else if items.length === 0}
        <div class="tray-empty">No background apps</div>
      {:else}
        <div class="tray-list">
          {#each items as item}
            <button
              class="tray-item"
              class:attention={item.status === "NeedsAttention"}
              onclick={(e) => { e.stopPropagation(); handleActivate(item.service); }}
            >
              <div class="tray-item-icon">{getInitials(item.id)}</div>
              <div class="tray-item-info">
                <span class="tray-item-title">{item.title}</span>
              </div>
              {#if item.status === "NeedsAttention"}
                <span class="tray-item-badge"></span>
              {/if}
            </button>
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
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    color: var(--color-fg-shell);
    display: flex; flex-direction: column;
    animation: pop-open 100ms ease-out both;
  }
  .pop-tray { right: 140px; width: 260px; }
  .pop-header { display: flex; align-items: center; gap: 8px; padding: 10px 12px; border-bottom: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .pop-title { flex: 1; font-size: 0.8125rem; font-weight: 500; }
  .pop-settings-btn { width: 24px; height: 24px; display: flex; align-items: center; justify-content: center; background: transparent; border: none; border-radius: 4px; color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent); cursor: pointer; padding: 0; }
  .pop-settings-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }
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
  .tray-item-info { flex: 1; min-width: 0; }
  .tray-item-title { font-size: 0.8125rem; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; display: block; }
  .tray-item-badge { width: 8px; height: 8px; background: #ef4444; border-radius: 50%; flex-shrink: 0; }
</style>
