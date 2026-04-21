<script lang="ts">
  /// Network status indicator for the top bar.
  ///
  /// Polls nmcli via Tauri every 5 seconds. Shows WiFi signal,
  /// Ethernet, or disconnected state. VPN shown as shield badge.

  import { invoke } from "@tauri-apps/api/core";
  import { togglePopover, hoverPopover } from "$lib/stores/activePopover.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import { Wifi, WifiOff, Cable, Shield, Plane } from "lucide-svelte";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

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
    // Run the two independent D-Bus queries in parallel. The extra
    // `get_network_status` call when airplane mode is on is cheap
    // (nmcli returns "disconnected" quickly) and saves ~100-200ms of
    // sequential waiting in the common non-airplane case.
    const [air, net] = await Promise.all([
      invoke<boolean>("get_airplane_mode").catch(() => false),
      invoke<NetworkStatus>("get_network_status").catch(() => null),
    ]);
    airplaneMode = air;
    status = air ? null : net;
  }

  poll();

  // Event-freshness fallback: `network-changed` is the authoritative
  // source. The timer below only fires if no event has arrived within
  // the freshness window, saving two D-Bus calls per 30s when the
  // event stream is healthy.
  const POLL_STALE_MS = 90_000;
  let lastEventAt = Date.now();

  onMount(() => {
    const unlisten = listen("network-changed", () => {
      lastEventAt = Date.now();
      poll();
    });
    const fallback = setInterval(() => {
      if (Date.now() - lastEventAt < POLL_STALE_MS) return;
      poll();
    }, 30_000);
    return () => {
      unlisten.then((fn) => fn());
      clearInterval(fallback);
    };
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
        onmouseenter={() => hoverPopover("network")}
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
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--foreground);
    transition:
      transform var(--duration-micro) var(--ease-out),
      background-color var(--duration-fast) var(--ease-out);
  }

  .network-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }

  .network-btn:active {
    transform: scale(0.96);
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
    border-radius: var(--radius-sm);
    color: var(--color-success);
  }
</style>
