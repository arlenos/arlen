<script lang="ts">
  /// Network popover: WiFi list with context menus, VPN, power toggle.
  /// Structure mirrors BluetoothPopover: Header > Sections > Items with ContextMenu.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import * as ContextMenu from "$lib/components/ui/context-menu/index.js";
  import {
    Wifi, WifiOff, Cable, Plane, Lock, Check, RefreshCw,
    ChevronRight, Shield, Trash2, Copy, Info,
  } from "lucide-svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import SignalBars from "$lib/components/SignalBars.svelte";

  interface WifiNetwork {
    ssid: string; signal: number; security: string;
    is_connected: boolean; is_known: boolean;
  }
  interface NetworkStatus {
    connection_type: string; connected: boolean; name: string | null;
    signal_strength: number | null; vpn_active: boolean;
  }
  interface VpnConnection { name: string; active: boolean; }
  interface ConnDetails { ip: string; gateway: string; dns: string; mac: string; }

  let status = $state<NetworkStatus | null>(null);
  let networks = $state<WifiNetwork[]>([]);
  let vpns = $state<VpnConnection[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let connectingTo = $state<string | null>(null);
  let showPasswordFor = $state<string | null>(null);
  let passwordInput = $state("");
  let airplaneMode = $state(false);
  let wifiEnabled = $state(true);
  let vpnExpanded = $state(false);
  let connDetails = $state<ConnDetails | null>(null);

  $effect(() => {
    if ($activePopover === "network") {
      checkAirplaneMode(); checkWifiEnabled();
      pollStatus(); loadNetworks(); loadVpns();
    } else {
      showPasswordFor = null; passwordInput = ""; error = null; connDetails = null;
    }
  });

  async function checkAirplaneMode() { try { airplaneMode = await invoke<boolean>("get_airplane_mode"); } catch {} }
  async function checkWifiEnabled() { try { wifiEnabled = await invoke<boolean>("get_wifi_enabled"); } catch {} }
  async function toggleWifi() {
    try {
      await invoke("set_wifi_enabled", { enabled: !wifiEnabled });
      wifiEnabled = !wifiEnabled;
      await pollStatus(); if (wifiEnabled) await loadNetworks();
    } catch { error = "Failed to toggle WiFi"; }
  }
  async function pollStatus() { try { status = await invoke<NetworkStatus>("get_network_status"); } catch {} }
  async function loadNetworks() {
    loading = true; error = null;
    try { networks = await invoke<WifiNetwork[]>("get_wifi_networks"); } catch { error = "Could not load networks"; }
    loading = false;
  }
  async function loadVpns() { try { vpns = await invoke<VpnConnection[]>("get_vpn_connections"); } catch {} }

  async function handleConnect(net: WifiNetwork) {
    if (net.is_connected) {
      try { await invoke("disconnect_wifi"); } catch { error = "Disconnect failed"; }
      await pollStatus(); await loadNetworks(); return;
    }
    if (net.is_known || !net.security || net.security === "--") {
      connectingTo = net.ssid;
      try { await invoke("connect_wifi", { ssid: net.ssid }); } catch { error = "Connection failed"; }
      connectingTo = null; await pollStatus(); await loadNetworks();
    } else { showPasswordFor = net.ssid; passwordInput = ""; }
  }
  async function handlePasswordSubmit() {
    if (!showPasswordFor || !passwordInput) return;
    connectingTo = showPasswordFor;
    try {
      await invoke("connect_wifi_password", { ssid: showPasswordFor, password: passwordInput });
      showPasswordFor = null; passwordInput = "";
    } catch { error = "Authentication failed"; }
    connectingTo = null; await pollStatus(); await loadNetworks();
  }
  async function copyPassword(ssid: string) {
    try { const pw = await invoke<string | null>("get_saved_password", { ssid }); if (pw) await navigator.clipboard.writeText(pw); } catch {}
  }
  async function copyText(text: string) { try { await navigator.clipboard.writeText(text); } catch {} }
  async function loadConnDetails(ssid: string) {
    try { connDetails = await invoke<ConnDetails>("get_connection_details", { ssid }); } catch { connDetails = null; }
  }
  async function forgetNetwork(ssid: string) {
    try { await invoke("forget_network", { ssid }); await loadNetworks(); } catch { error = "Could not forget network"; }
  }
  async function toggleVpn(vpn: VpnConnection) {
    try {
      if (vpn.active) await invoke("disconnect_vpn", { name: vpn.name });
      else await invoke("connect_vpn", { name: vpn.name });
      await loadVpns(); await pollStatus();
    } catch { error = "VPN operation failed"; }
  }

  const connectedNets = $derived(networks.filter(n => n.is_connected));
  const otherNets = $derived(networks.filter(n => !n.is_connected));
  const activeVpnCount = $derived(vpns.filter(v => v.active).length);
</script>

{#snippet networkItem(net: WifiNetwork)}
  <ContextMenu.Root>
    <ContextMenu.Trigger>
      {#snippet child({ props })}
        <button
          {...props}
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
      {/snippet}
    </ContextMenu.Trigger>
    <ContextMenu.Content class="shell-popover">
      {#if net.is_connected}
        <ContextMenu.Item onclick={() => handleConnect(net)}>
          <WifiOff size={14} class="mr-2" />Disconnect
        </ContextMenu.Item>
      {:else}
        <ContextMenu.Item onclick={() => handleConnect(net)}>
          <Wifi size={14} class="mr-2" />Connect
        </ContextMenu.Item>
      {/if}
      {#if net.is_known}
        <ContextMenu.Item onclick={() => copyPassword(net.ssid)}>
          <Copy size={14} class="mr-2" />Copy Password
        </ContextMenu.Item>
      {/if}
      {#if net.is_connected}
        <ContextMenu.Separator />
        <ContextMenu.Sub>
          <ContextMenu.SubTrigger>
            <Info size={14} class="mr-2" />Connection Info
          </ContextMenu.SubTrigger>
          <ContextMenu.SubContent class="shell-popover">
            {#if connDetails}
              {#if connDetails.ip}
                <ContextMenu.Item onclick={() => copyText(connDetails!.ip)}>
                  <span class="ctx-label">IP</span><span class="ctx-value">{connDetails.ip}</span>
                </ContextMenu.Item>
              {/if}
              {#if connDetails.gateway}
                <ContextMenu.Item onclick={() => copyText(connDetails!.gateway)}>
                  <span class="ctx-label">GW</span><span class="ctx-value">{connDetails.gateway}</span>
                </ContextMenu.Item>
              {/if}
              {#if connDetails.dns}
                <ContextMenu.Item onclick={() => copyText(connDetails!.dns)}>
                  <span class="ctx-label">DNS</span><span class="ctx-value">{connDetails.dns}</span>
                </ContextMenu.Item>
              {/if}
              {#if connDetails.mac}
                <ContextMenu.Item onclick={() => copyText(connDetails!.mac)}>
                  <span class="ctx-label">MAC</span><span class="ctx-value">{connDetails.mac}</span>
                </ContextMenu.Item>
              {/if}
            {:else}
              <ContextMenu.Item onclick={() => loadConnDetails(net.ssid)}>
                Load details...
              </ContextMenu.Item>
            {/if}
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
      {/if}
      {#if net.is_known}
        <ContextMenu.Separator />
        <ContextMenu.Item onclick={() => forgetNetwork(net.ssid)} class="text-red-400">
          <Trash2 size={14} class="mr-2" />Forget Network
        </ContextMenu.Item>
      {/if}
    </ContextMenu.Content>
  </ContextMenu.Root>
{/snippet}

{#if $activePopover === "network"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-backdrop" onclick={closePopover}></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-panel pop-network shell-popover" onclick={(e) => e.stopPropagation()}>

    <PopoverHeader icon={Wifi} title="Network" toggled={wifiEnabled && !airplaneMode} onToggle={toggleWifi} />

    <div class="pop-body">
      {#if airplaneMode}
        <div class="net-msg">
          <Plane size={32} strokeWidth={1} />
          <span class="net-msg-title">Airplane Mode is on</span>
          <span class="net-msg-hint">Wireless connections are disabled</span>
        </div>
      {:else if !wifiEnabled}
        <div class="net-msg">
          <WifiOff size={32} strokeWidth={1} />
          <span class="net-msg-title">WiFi is off</span>
          <span class="net-msg-hint">Toggle the switch above to enable</span>
        </div>
      {:else}
        {#if error}
          <div class="net-error">{error}</div>
        {/if}

        <!-- Current connection status (always visible when connected) -->
        {#if status?.connected}
          <div class="net-status">
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
          </div>
          <Separator class="opacity-10" />
        {:else}
          <div class="net-status">
            <div class="net-status-icon net-off"><WifiOff size={18} strokeWidth={1.5} /></div>
            <div class="net-status-info">
              <span class="net-status-name">Disconnected</span>
            </div>
          </div>
          <Separator class="opacity-10" />
        {/if}

        {#if showPasswordFor}
          <div class="pw-section">
            <span class="pw-title">Connect to "{showPasswordFor}"</span>
            <input type="password" class="pw-input" bind:value={passwordInput} placeholder="Password"
              onkeydown={(e) => { if (e.key === "Enter") handlePasswordSubmit(); }} />
            <div class="pw-actions">
              <button class="pw-btn" onclick={(e) => { e.stopPropagation(); showPasswordFor = null; }}>Cancel</button>
              <button class="pw-btn pw-btn-primary" onclick={(e) => { e.stopPropagation(); handlePasswordSubmit(); }}>Connect</button>
            </div>
          </div>
        {:else if loading}
          <div class="net-loading">Scanning...</div>
        {:else}
          <div class="net-list-header">
            <span>Available Networks</span>
            <button class="net-refresh" onclick={(e) => { e.stopPropagation(); loadNetworks(); }} title="Refresh">
              <RefreshCw size={12} strokeWidth={2} />
            </button>
          </div>
          <div class="net-list">
            {#each otherNets as net}
              {@render networkItem(net)}
            {:else}
              <div class="net-empty">No networks found</div>
            {/each}
          </div>
        {/if}

        {#if status?.connection_type === "ethernet" && status.connected}
          <Separator class="opacity-10" />
          <div class="net-ethernet">
            <Cable size={14} strokeWidth={1.5} />
            <span>{status.name ?? "Ethernet"}</span>
            <span class="net-ethernet-badge">Connected</span>
          </div>
        {/if}

        {#if vpns.length > 0}
          <Separator class="opacity-10" />
          <button class="vpn-header" onclick={(e) => { e.stopPropagation(); vpnExpanded = !vpnExpanded; }}>
            <ChevronRight size={12} strokeWidth={2} class={vpnExpanded ? "vpn-chevron-open" : ""} />
            <Shield size={14} strokeWidth={1.5} />
            <span>VPN</span>
            {#if activeVpnCount > 0}
              <span class="vpn-badge">{activeVpnCount} active</span>
            {/if}
          </button>
          {#if vpnExpanded}
            <div class="vpn-list">
              {#each vpns as vpn}
                <button class="net-item" class:connected={vpn.active}
                  onclick={(e) => { e.stopPropagation(); toggleVpn(vpn); }}>
                  <div class="net-item-info">
                    {#if vpn.active}<Check size={14} strokeWidth={2} class="net-check" />{/if}
                    <span>{vpn.name}</span>
                  </div>
                  <span class="vpn-status">{vpn.active ? "Connected" : "Connect"}</span>
                </button>
              {/each}
            </div>
          {/if}
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
    box-shadow: var(--shadow-lg); color: var(--color-fg-shell);
    display: flex; flex-direction: column;
    animation: pop-open 100ms ease-out both;
  }
  .pop-network { right: 110px; width: 280px; }
  .pop-body { padding: 12px; display: flex; flex-direction: column; gap: 6px; }
  @keyframes pop-open { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }

  .net-msg { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 24px 12px; color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent); text-align: center; font-size: 0.8125rem; }
  .net-msg-title { color: var(--color-fg-shell); }
  .net-msg-hint { font-size: 0.6875rem; opacity: 0.5; }
  .net-status { display: flex; align-items: center; gap: 10px; }
  .net-status-icon { opacity: 0.7; }
  .net-off { opacity: 0.3; }
  .net-status-info { display: flex; flex-direction: column; gap: 1px; }
  .net-status-name { font-size: 0.8125rem; font-weight: 500; }
  .net-status-detail { font-size: 0.6875rem; opacity: 0.5; }

  .net-error { padding: 6px 10px; background: rgba(239, 68, 68, 0.15); border-radius: 6px; color: #ef4444; font-size: 0.6875rem; }
  .net-loading { padding: 20px; text-align: center; opacity: 0.4; font-size: 0.75rem; }

  .net-section-label { font-size: 0.6875rem; opacity: 0.5; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
  .net-list-header { display: flex; align-items: center; justify-content: space-between; font-size: 0.6875rem; opacity: 0.5; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }
  .net-refresh { width: 20px; height: 20px; display: flex; align-items: center; justify-content: center; background: transparent; border: none; border-radius: 4px; color: inherit; cursor: pointer; padding: 0; }
  .net-refresh:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }

  .net-list { display: flex; flex-direction: column; gap: 2px; max-height: 200px; overflow-y: auto; }
  .net-item {
    display: flex; align-items: center; justify-content: space-between;
    padding: 8px 10px; background: transparent; border: none; border-radius: 6px;
    color: var(--color-fg-shell); font-size: 0.8125rem; cursor: pointer;
    text-align: left; width: 100%; transition: background-color 0.1s ease;
  }
  .net-item:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .net-item.connected { background: color-mix(in srgb, var(--color-accent) 15%, transparent); border: 1px solid color-mix(in srgb, var(--color-accent) 30%, transparent); }
  .net-item.connecting { opacity: 0.5; pointer-events: none; }
  .net-item-info { display: flex; align-items: center; gap: 8px; }
  :global(.net-check) { color: #22c55e; }
  .net-item-meta { display: flex; align-items: center; gap: 6px; opacity: 0.5; }
  .net-empty { padding: 20px; text-align: center; opacity: 0.3; font-size: 0.75rem; }

  :global(.ctx-label) { opacity: 0.5; font-size: 0.625rem; min-width: 32px; text-transform: uppercase; letter-spacing: 0.03em; }
  :global(.ctx-value) { font-size: 0.6875rem; font-family: monospace; }

  .net-ethernet { display: flex; align-items: center; gap: 8px; padding: 6px 10px; font-size: 0.8125rem; opacity: 0.7; }
  .net-ethernet-badge { margin-left: auto; font-size: 0.6875rem; opacity: 0.5; }

  .vpn-header {
    display: flex; align-items: center; gap: 6px;
    padding: 6px 4px; background: transparent; border: none; border-radius: 4px;
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    font-size: 0.75rem; font-weight: 500; cursor: pointer; width: 100%; text-align: left;
    transition: color 0.1s ease;
  }
  .vpn-header:hover { color: var(--color-fg-shell); }
  :global(.vpn-chevron-open) { transform: rotate(90deg); }
  .vpn-badge { margin-left: auto; font-size: 0.625rem; opacity: 0.5; font-weight: 400; }
  .vpn-list { display: flex; flex-direction: column; gap: 2px; }
  .vpn-status { font-size: 0.6875rem; opacity: 0.5; }

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
  .pw-btn-primary { background: var(--color-accent); color: var(--color-accent-fg, var(--color-bg-shell)); border: none; font-weight: 500; }
  .pw-btn-primary:hover { opacity: 0.9; }
</style>
