<script lang="ts">
  /// Network status indicator for the top bar.
  ///
  /// Polls nmcli via Tauri every 5 seconds. Shows WiFi signal,
  /// Ethernet, or disconnected state. VPN shown as shield badge.

  import { invoke } from "@tauri-apps/api/core";
  import { togglePopover } from "$lib/stores/activePopover.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import { Wifi, WifiOff, Cable, Shield, Plane } from "lucide-svelte";

  interface NetworkStatus {
    connection_type: string;
    connected: boolean;
    name: string | null;
    signal_strength: number | null;
    vpn_active: boolean;
  }

  let status = $state<NetworkStatus | null>(null);
  let airplaneMode = $state(false);

  const tooltipClass =
    "rounded-md border px-2 py-0.5 text-xs shadow-md select-none"
    + " bg-[var(--color-bg-shell)] text-[var(--color-fg-shell)] border-[color-mix(in_srgb,var(--color-bg-shell)_60%,white_40%)]";

  async function poll() {
    try {
      airplaneMode = await invoke<boolean>("get_airplane_mode");
    } catch {
      airplaneMode = false;
    }
    if (!airplaneMode) {
      try {
        status = await invoke<NetworkStatus>("get_network_status");
      } catch {
        status = null;
      }
    }
  }

  poll();
  let _interval: ReturnType<typeof setInterval> | null = null;
  $effect(() => {
    if (_interval) return;
    _interval = setInterval(poll, 5_000);
    return () => { if (_interval) { clearInterval(_interval); _interval = null; } };
  });

  const Icon = $derived(
    airplaneMode ? Plane :
    !status || !status.connected ? WifiOff :
    status.connection_type === "ethernet" ? Cable :
    Wifi
  );

  // WiFi signal opacity: stronger signal = more opaque icon.
  const signalOpacity = $derived(
    status?.signal_strength != null
      ? Math.max(0.4, status.signal_strength / 100)
      : 1
  );

  const label = $derived(() => {
    if (airplaneMode) return "Airplane Mode";
    if (!status || !status.connected) return "Disconnected";
    if (status.connection_type === "ethernet") {
      return `Ethernet: ${status.name ?? "Connected"}`;
    }
    let text = `WiFi: ${status.name ?? "Connected"}`;
    if (status.signal_strength != null) {
      text += ` (${status.signal_strength}%)`;
    }
    if (status.vpn_active) {
      text += " · VPN";
    }
    return text;
  });

  function handleContextMenu(e: MouseEvent) {
    e.preventDefault();
    // TODO: Open Network Settings.
  }
</script>

<Tooltip.Root>
  <Tooltip.Trigger>
    {#snippet child({ props })}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <button
        class="network-btn"
        class:disconnected={!status?.connected}
        aria-label={label()}
        {...props}
        onclick={() => togglePopover("network")}
        oncontextmenu={handleContextMenu}
      >
        <span style:opacity={signalOpacity}>
          <Icon size={14} strokeWidth={1.5} />
        </span>
        {#if status?.vpn_active}
          <span class="vpn-badge">
            <Shield size={7} strokeWidth={2.5} />
          </span>
        {/if}
      </button>
    {/snippet}
  </Tooltip.Trigger>
  <Tooltip.Portal>
    <Tooltip.Content side="bottom" class={tooltipClass}>{label()}</Tooltip.Content>
  </Tooltip.Portal>
</Tooltip.Root>

<style>
  .network-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
    min-width: 24px;
    min-height: 24px;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: 4px;
    cursor: pointer;
    color: var(--foreground);
    transition: background-color var(--duration-fast, 150ms) ease;
  }

  .network-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .disconnected {
    opacity: 0.4;
  }

  .vpn-badge {
    position: absolute;
    bottom: 2px;
    right: 2px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 10px;
    height: 10px;
    background: var(--color-bg-shell);
    border-radius: 2px;
    color: var(--color-success);
  }
</style>
