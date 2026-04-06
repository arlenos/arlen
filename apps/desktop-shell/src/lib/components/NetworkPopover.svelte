<script lang="ts">
  /// Network popover: current connection + available networks.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { Wifi, WifiOff, Cable, Settings } from "lucide-svelte";

  interface NetworkStatus {
    connection_type: string;
    connected: boolean;
    name: string | null;
    signal_strength: number | null;
    vpn_active: boolean;
  }

  let status = $state<NetworkStatus | null>(null);

  async function poll() {
    try {
      status = await invoke<NetworkStatus>("get_network_status");
    } catch {}
  }

  // Fetch data when popover opens. Read $activePopover directly
  // so Svelte 5 tracks the dependency.
  $effect(() => {
    if ($activePopover === "network") poll();
  });
</script>

{#if $activePopover === "network"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-backdrop" onclick={closePopover}></div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-panel pop-network shell-popover" onclick={(e) => e.stopPropagation()}>
    <div class="pop-header">
      <Wifi size={16} strokeWidth={1.5} />
      <span class="pop-title">Network</span>
      <button class="pop-settings-btn" onclick={(e) => { e.stopPropagation(); closePopover(); }}>
        <Settings size={14} strokeWidth={1.5} />
      </button>
    </div>

    <div class="pop-body">
    <div class="net-current">
      {#if status?.connected}
        <div class="net-icon">
          {#if status.connection_type === "ethernet"}
            <Cable size={20} strokeWidth={1.5} />
          {:else}
            <Wifi size={20} strokeWidth={1.5} />
          {/if}
        </div>
        <div class="net-info">
          <span class="net-name">{status.name ?? "Connected"}</span>
          <span class="net-type">
            {#if status.signal_strength != null}{status.signal_strength}% · {/if}{status.connection_type === "ethernet" ? "Ethernet" : "WiFi"}{#if status.vpn_active} · VPN{/if}
          </span>
        </div>
      {:else}
        <div class="net-icon net-off">
          <WifiOff size={20} strokeWidth={1.5} />
        </div>
        <div class="net-info">
          <span class="net-name">Disconnected</span>
          <span class="net-type">No network connection</span>
        </div>
      {/if}
    </div>
    </div>
  </div>
{/if}

<style>
  .pop-backdrop { position: fixed; inset: 0; z-index: 90; }

  .pop-panel {
    position: fixed;
    top: 40px;
    z-index: 100;
    border-radius: 10px;
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.4);
    color: var(--color-fg-shell);
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    animation: pop-open 100ms ease-out both;
  }

  .pop-network { right: 110px; width: 260px; padding: 0; }

  .pop-header {
    display: flex; align-items: center; gap: 8px; padding: 10px 12px;
    border-bottom: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }
  .pop-title { flex: 1; font-size: 0.8125rem; font-weight: 500; }
  .pop-settings-btn {
    width: 24px; height: 24px; display: flex; align-items: center; justify-content: center;
    background: transparent; border: none; border-radius: 4px;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent); cursor: pointer;
    transition: all 0.1s ease;
  }
  .pop-settings-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }

  @keyframes pop-open {
    from { opacity: 0; transform: translateY(-4px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .pop-body { padding: 12px; }
  .net-current { display: flex; align-items: center; gap: 12px; }
  .net-icon { opacity: 0.7; }
  .net-off { opacity: 0.3; }
  .net-info { display: flex; flex-direction: column; gap: 1px; }
  .net-name { font-size: 0.8125rem; font-weight: 500; }
  .net-type { font-size: 0.6875rem; opacity: 0.5; }

</style>
