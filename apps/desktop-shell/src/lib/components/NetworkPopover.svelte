<script lang="ts">
  /// Network popover: status, WiFi list, connect/disconnect.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { Wifi, WifiOff, Cable, Plane, Lock, Check, RefreshCw } from "lucide-svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import SignalBars from "$lib/components/SignalBars.svelte";

  interface NetworkStatus {
    connection_type: string;
    connected: boolean;
    name: string | null;
    signal_strength: number | null;
    vpn_active: boolean;
  }

  interface WifiNetwork {
    ssid: string;
    signal: number;
    security: string;
    is_connected: boolean;
    is_known: boolean;
  }

  let status = $state<NetworkStatus | null>(null);
  let networks = $state<WifiNetwork[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let connectingTo = $state<string | null>(null);
  let showPasswordFor = $state<string | null>(null);
  let passwordInput = $state("");
  let airplaneMode = $state(false);

  $effect(() => {
    if ($activePopover === "network") {
      checkAirplaneMode();
      pollStatus();
      loadNetworks();
    } else {
      showPasswordFor = null;
      passwordInput = "";
      error = null;
    }
  });

  async function checkAirplaneMode() {
    try { airplaneMode = await invoke<boolean>("get_airplane_mode"); } catch {}
  }

  async function pollStatus() {
    try { status = await invoke<NetworkStatus>("get_network_status"); } catch {}
  }

  async function loadNetworks() {
    loading = true;
    error = null;
    try { networks = await invoke<WifiNetwork[]>("get_wifi_networks"); }
    catch { error = "Could not load networks"; }
    loading = false;
  }

  async function handleConnect(net: WifiNetwork) {
    if (net.is_connected) {
      try { await invoke("disconnect_wifi"); } catch { error = "Disconnect failed"; }
      await pollStatus();
      await loadNetworks();
      return;
    }
    if (net.is_known || !net.security || net.security === "--") {
      connectingTo = net.ssid;
      try { await invoke("connect_wifi", { ssid: net.ssid }); }
      catch { error = "Connection failed"; }
      connectingTo = null;
      await pollStatus();
      await loadNetworks();
    } else {
      showPasswordFor = net.ssid;
      passwordInput = "";
    }
  }

  async function handlePasswordSubmit() {
    if (!showPasswordFor || !passwordInput) return;
    connectingTo = showPasswordFor;
    try {
      await invoke("connect_wifi_password", { ssid: showPasswordFor, password: passwordInput });
      showPasswordFor = null;
      passwordInput = "";
    } catch { error = "Authentication failed"; }
    connectingTo = null;
    await pollStatus();
    await loadNetworks();
  }
</script>

{#if $activePopover === "network"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-backdrop" onclick={closePopover}></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-panel pop-network shell-popover" onclick={(e) => e.stopPropagation()}>

    <PopoverHeader icon={Wifi} title="Network" />

    <div class="pop-body">
      {#if airplaneMode}
        <div class="airplane-msg">
          <Plane size={32} strokeWidth={1} />
          <span class="airplane-title">Airplane Mode is on</span>
          <span class="airplane-hint">Wireless connections are disabled</span>
        </div>
      {:else}
      <!-- Current Status -->
      <div class="net-status">
        {#if status?.connected}
          <div class="net-status-icon">
            {#if status.connection_type === "ethernet"}
              <Cable size={18} strokeWidth={1.5} />
            {:else}
              <Wifi size={18} strokeWidth={1.5} />
            {/if}
          </div>
          <div class="net-status-info">
            <span class="net-status-name">{status.name ?? "Connected"}</span>
            <span class="net-status-detail">
              {#if status.signal_strength != null}{status.signal_strength}% · {/if}{status.connection_type === "ethernet" ? "Ethernet" : "WiFi"}{#if status.vpn_active} · VPN{/if}
            </span>
          </div>
        {:else}
          <div class="net-status-icon net-off"><WifiOff size={18} strokeWidth={1.5} /></div>
          <div class="net-status-info">
            <span class="net-status-name">Disconnected</span>
          </div>
        {/if}
      </div>

      {#if error}
        <div class="net-error">{error}</div>
      {/if}

      <Separator class="opacity-10" />

      {#if showPasswordFor}
        <!-- Password Input -->
        <div class="pw-section">
          <span class="pw-title">Connect to "{showPasswordFor}"</span>
          <input
            type="password"
            class="pw-input"
            bind:value={passwordInput}
            placeholder="Password"
            onkeydown={(e) => { if (e.key === "Enter") handlePasswordSubmit(); }}
          />
          <div class="pw-actions">
            <button class="pw-btn" onclick={(e) => { e.stopPropagation(); showPasswordFor = null; }}>Cancel</button>
            <button class="pw-btn pw-btn-primary" onclick={(e) => { e.stopPropagation(); handlePasswordSubmit(); }}>Connect</button>
          </div>
        </div>
      {:else if loading}
        <div class="net-loading">Scanning...</div>
      {:else}
        <!-- WiFi List -->
        <div class="net-list-header">
          <span>Available Networks</span>
          <button class="net-refresh" onclick={(e) => { e.stopPropagation(); loadNetworks(); }} title="Refresh">
            <RefreshCw size={12} strokeWidth={2} />
          </button>
        </div>
        <div class="net-list">
          {#each networks as net}
            <button
              class="net-item"
              class:connected={net.is_connected}
              class:connecting={connectingTo === net.ssid}
              onclick={(e) => { e.stopPropagation(); handleConnect(net); }}
            >
              <div class="net-item-info">
                {#if net.is_connected}<Check size={14} strokeWidth={2} class="net-check" />{/if}
                <span>{net.ssid}</span>
              </div>
              <div class="net-item-meta">
                <SignalBars signal={net.signal} />
                {#if net.security && net.security !== "--"}
                  <Lock size={10} strokeWidth={2} />
                {/if}
              </div>
            </button>
          {:else}
            <div class="net-empty">No networks found</div>
          {/each}
        </div>
      {/if}
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
  .pop-network { right: 110px; width: 280px; }
  .pop-body { padding: 12px; display: flex; flex-direction: column; gap: 8px; }
  @keyframes pop-open { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }

  /* Status */
  .net-status { display: flex; align-items: center; gap: 10px; }
  .net-status-icon { opacity: 0.7; }
  .net-off { opacity: 0.3; }
  .net-status-info { display: flex; flex-direction: column; gap: 1px; }
  .net-status-name { font-size: 0.8125rem; font-weight: 500; }
  .net-status-detail { font-size: 0.6875rem; opacity: 0.5; }

  .airplane-msg { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 24px 12px; color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent); text-align: center; }
  .airplane-title { font-size: 0.8125rem; color: var(--color-fg-shell); }
  .airplane-hint { font-size: 0.6875rem; }

  .net-error { padding: 6px 10px; background: rgba(239, 68, 68, 0.15); border-radius: 6px; color: #ef4444; font-size: 0.6875rem; }
  .net-loading { padding: 20px; text-align: center; opacity: 0.4; font-size: 0.75rem; }

  /* List */
  .net-list-header { display: flex; align-items: center; justify-content: space-between; font-size: 0.6875rem; opacity: 0.5; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
  .net-refresh { width: 20px; height: 20px; display: flex; align-items: center; justify-content: center; background: transparent; border: none; border-radius: 4px; color: inherit; cursor: pointer; padding: 0; }
  .net-refresh:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }

  .net-list { display: flex; flex-direction: column; gap: 2px; max-height: 200px; overflow-y: auto; }
  .net-item {
    display: flex; align-items: center; justify-content: space-between;
    padding: 7px 10px; background: transparent; border: none; border-radius: 6px;
    color: var(--color-fg-shell); font-size: 0.8125rem; cursor: pointer;
    text-align: left; width: 100%; transition: background-color 0.1s ease;
  }
  .net-item:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .net-item.connected { background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent); }
  .net-item.connecting { opacity: 0.5; pointer-events: none; }
  .net-item-info { display: flex; align-items: center; gap: 8px; }
  :global(.net-check) { color: #22c55e; }
  .net-item-meta { display: flex; align-items: center; gap: 6px; opacity: 0.5; }
  .net-empty { padding: 20px; text-align: center; opacity: 0.3; font-size: 0.75rem; }

  /* Password */
  .pw-section { display: flex; flex-direction: column; gap: 10px; }
  .pw-title { font-size: 0.8125rem; }
  .pw-input {
    width: 100%; padding: 7px 10px; border-radius: 6px;
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    color: var(--color-fg-shell); font-size: 0.8125rem; outline: none;
  }
  .pw-input:focus { border-color: color-mix(in srgb, var(--color-fg-shell) 40%, transparent); }
  .pw-actions { display: flex; justify-content: flex-end; gap: 6px; }
  .pw-btn {
    padding: 5px 12px; border-radius: 6px; font-size: 0.6875rem; cursor: pointer;
    background: transparent; border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    color: var(--color-fg-shell);
  }
  .pw-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .pw-btn-primary { background: var(--color-fg-shell); color: var(--color-bg-shell); border: none; font-weight: 500; }
  .pw-btn-primary:hover { opacity: 0.9; }
</style>
