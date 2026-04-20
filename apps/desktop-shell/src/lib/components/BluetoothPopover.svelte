<script lang="ts">
  /// Bluetooth popover: device list with context menus, scan, power toggle.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import * as ContextMenu from "$lib/components/ui/context-menu/index.js";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import {
    Bluetooth, BluetoothOff, RefreshCw, BatteryMedium,
    Headphones, Keyboard, Mouse, Gamepad2, Smartphone, Speaker,
    Plug, Unplug, Trash2, ShieldOff, ShieldCheck, Loader2,
  } from "lucide-svelte";

  interface BluetoothDevice {
    path: string;
    address: string;
    name: string;
    icon: string;
    paired: boolean;
    connected: boolean;
    trusted: boolean;
    battery_percentage: number | null;
  }

  interface BluetoothState {
    available: boolean;
    powered: boolean;
    discovering: boolean;
    devices: BluetoothDevice[];
  }

  let btState = $state<BluetoothState | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let connectingTo = $state<string | null>(null);

  const iconMap: Record<string, typeof Bluetooth> = {
    "audio-headphones": Headphones,
    "audio-headset": Headphones,
    "audio-speakers": Speaker,
    "input-keyboard": Keyboard,
    "input-mouse": Mouse,
    "input-gaming": Gamepad2,
    "phone": Smartphone,
  };

  function deviceIcon(icon: string) {
    return iconMap[icon] ?? Bluetooth;
  }

  async function load() {
    loading = true;
    error = null;
    try {
      btState = await invoke<BluetoothState>("get_bluetooth_state");
    } catch {
      error = "Could not load Bluetooth state";
    }
    loading = false;
  }

  let scanTimer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if ($activePopover === "bluetooth") {
      load();
    } else {
      if (btState?.discovering) {
        invoke("stop_bluetooth_scan").catch(() => {});
      }
      if (scanTimer) { clearTimeout(scanTimer); scanTimer = null; }
      error = null;
      connectingTo = null;
    }
  });

  onMount(() => {
    const unlisten = listen("bluetooth-changed", () => {
      if ($activePopover === "bluetooth") load();
    });
    return () => { unlisten.then((fn) => fn()); };
  });

  async function togglePower() {
    if (!btState) return;
    try {
      await invoke("set_bluetooth_powered", { enabled: !btState.powered });
      await load();
    } catch {
      error = "Could not toggle Bluetooth";
    }
  }

  async function toggleScan() {
    if (!btState) return;
    try {
      if (btState.discovering) {
        await invoke("stop_bluetooth_scan");
        if (scanTimer) { clearTimeout(scanTimer); scanTimer = null; }
      } else {
        await invoke("start_bluetooth_scan");
        // Auto-stop after 10 seconds.
        scanTimer = setTimeout(async () => {
          try { await invoke("stop_bluetooth_scan"); } catch {}
          scanTimer = null;
          await load();
        }, 10_000);
      }
      await load();
    } catch {}
  }

  async function handleClick(dev: BluetoothDevice) {
    error = null;
    if (dev.connected) {
      try { await invoke("disconnect_bluetooth_device", { path: dev.path }); }
      catch { error = "Disconnect failed"; }
      await load();
      return;
    }
    connectingTo = dev.path;
    try {
      if (!dev.paired) await invoke("pair_bluetooth_device", { path: dev.path });
      await invoke("connect_bluetooth_device", { path: dev.path });
    } catch {
      error = "Connection failed";
    }
    connectingTo = null;
    await load();
  }

  async function disconnect(path: string) {
    try { await invoke("disconnect_bluetooth_device", { path }); } catch {}
    await load();
  }

  async function connect(path: string) {
    connectingTo = path;
    try { await invoke("connect_bluetooth_device", { path }); } catch { error = "Connection failed"; }
    connectingTo = null;
    await load();
  }

  async function setTrusted(path: string, trusted: boolean) {
    try { await invoke("set_device_trusted", { path, trusted }); } catch {}
    await load();
  }

  async function remove(path: string) {
    try { await invoke("remove_bluetooth_device", { path }); } catch { error = "Could not remove device"; }
    await load();
  }

  const connectedDevices = $derived(
    btState?.devices.filter((d: BluetoothDevice) => d.connected) ?? []
  );
  const pairedDevices = $derived(
    btState?.devices.filter((d: BluetoothDevice) => d.paired && !d.connected) ?? []
  );
  const availableDevices = $derived(
    btState?.devices.filter((d: BluetoothDevice) => !d.paired && !d.connected) ?? []
  );
</script>

{#snippet devIcon(iconName: string)}
  {#if iconName === "audio-headphones" || iconName === "audio-headset"}
    <Headphones size={16} strokeWidth={1.5} />
  {:else if iconName === "audio-speakers"}
    <Speaker size={16} strokeWidth={1.5} />
  {:else if iconName === "input-keyboard"}
    <Keyboard size={16} strokeWidth={1.5} />
  {:else if iconName === "input-mouse"}
    <Mouse size={16} strokeWidth={1.5} />
  {:else if iconName === "input-gaming"}
    <Gamepad2 size={16} strokeWidth={1.5} />
  {:else if iconName === "phone"}
    <Smartphone size={16} strokeWidth={1.5} />
  {:else}
    <Bluetooth size={16} strokeWidth={1.5} />
  {/if}
{/snippet}

{#snippet deviceItem(dev: BluetoothDevice)}
  <ContextMenu.Root>
    <ContextMenu.Trigger>
      {#snippet child({ props })}
        <button
          {...props}
          class="bt-device"
          class:connected={dev.connected}
          class:connecting={connectingTo === dev.path}
          onclick={(e) => { e.stopPropagation(); handleClick(dev); }}
        >
          <div class="bt-device-icon">
            {#if connectingTo === dev.path}
              <Loader2 size={16} strokeWidth={1.5} class="spinning" />
            {:else}
              {@render devIcon(dev.icon)}
            {/if}
          </div>
          <div class="bt-device-info">
            <span class="bt-device-name">{dev.name}</span>
            <span class="bt-device-detail">
              {#if connectingTo === dev.path}
                Connecting...
              {:else if dev.connected}
                Connected
              {:else if dev.paired}
                Paired
              {/if}
              {#if dev.battery_percentage != null}
                {#if dev.connected || dev.paired}
                  &middot;
                {/if}
                <BatteryMedium size={12} strokeWidth={1.5} class="bt-battery-icon" />
                {dev.battery_percentage}%
              {/if}
            </span>
          </div>
        </button>
      {/snippet}
    </ContextMenu.Trigger>
    <ContextMenu.Content class="shell-popover">
      {#if dev.connected}
        <ContextMenu.Item onclick={() => disconnect(dev.path)}>
          <Unplug size={14} class="mr-2" />Disconnect
        </ContextMenu.Item>
      {:else}
        <ContextMenu.Item onclick={() => connect(dev.path)}>
          <Plug size={14} class="mr-2" />Connect
        </ContextMenu.Item>
      {/if}
      <ContextMenu.Separator />
      {#if dev.trusted}
        <ContextMenu.Item onclick={() => setTrusted(dev.path, false)}>
          <ShieldOff size={14} class="mr-2" />Don't Auto-Connect
        </ContextMenu.Item>
      {:else}
        <ContextMenu.Item onclick={() => setTrusted(dev.path, true)}>
          <ShieldCheck size={14} class="mr-2" />Auto-Connect
        </ContextMenu.Item>
      {/if}
      {#if dev.paired}
        <ContextMenu.Separator />
        <ContextMenu.Item onclick={() => remove(dev.path)} class="text-red-400">
          <Trash2 size={14} class="mr-2" />Forget Device
        </ContextMenu.Item>
      {/if}
    </ContextMenu.Content>
  </ContextMenu.Root>
{/snippet}

{#if $activePopover === "bluetooth"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-backdrop" onclick={closePopover}></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-panel pop-bt shell-popover" onclick={(e) => e.stopPropagation()}>

    <PopoverHeader
      icon={Bluetooth}
      title="Bluetooth"
      toggled={btState?.powered ?? false}
      onToggle={togglePower}
    />

    <div class="pop-body">
      {#if !btState?.available}
        <div class="bt-msg">
          <BluetoothOff size={32} strokeWidth={1} />
          <span>No Bluetooth adapter</span>
        </div>
      {:else if !btState.powered}
        <div class="bt-msg">
          <BluetoothOff size={32} strokeWidth={1} />
          <span>Bluetooth is off</span>
          <span class="bt-hint">Toggle the switch above to enable</span>
        </div>
      {:else}
        {#if error}
          <div class="bt-error">{error}</div>
        {/if}

        {#if connectedDevices.length > 0}
          <div class="bt-section-label">Connected</div>
          {#each connectedDevices as dev (dev.address)}
            {@render deviceItem(dev)}
          {/each}
          <Separator class="opacity-10" />
        {/if}

        {#if pairedDevices.length > 0}
          <div class="bt-section-label">Paired Devices</div>
          {#each pairedDevices as dev (dev.address)}
            {@render deviceItem(dev)}
          {/each}
          <Separator class="opacity-10" />
        {/if}

        {#if btState.discovering && availableDevices.length > 0}
          <div class="bt-section-label">Available</div>
          {#each availableDevices as dev (dev.address)}
            {@render deviceItem(dev)}
          {/each}
          <Separator class="opacity-10" />
        {/if}

        <button class="bt-scan-btn" onclick={(e) => { e.stopPropagation(); toggleScan(); }}>
          <RefreshCw size={12} strokeWidth={2} class={btState.discovering ? "spinning" : ""} />
          <span>{btState.discovering ? "Scanning..." : "Scan for Devices"}</span>
        </button>
      {/if}
    </div>
  </div>
{/if}

<style>
  .pop-backdrop { position: fixed; inset: 0; z-index: 90; }
  .pop-panel {
    position: fixed; top: 40px; z-index: 100; border-radius: var(--radius-lg);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-shell);
    display: flex; flex-direction: column;
    animation: lunaris-popover-in var(--duration-medium) var(--ease-out) both;
    transform-origin: top center;
  }
  .pop-bt { right: 80px; width: 280px; }
  .pop-body { padding: 12px; display: flex; flex-direction: column; gap: 6px; }
  /* Entry keyframes defined in sdk/ui-kit/src/lib/motion.css. */

  .bt-msg { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 24px 12px; color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent); text-align: center; font-size: 0.8125rem; }
  .bt-hint { font-size: 0.6875rem; opacity: 0.5; }

  .bt-error { padding: 6px 10px; background: rgba(239, 68, 68, 0.15); border-radius: var(--radius-md); color: #ef4444; font-size: 0.6875rem; }

  .bt-section-label { font-size: 0.6875rem; opacity: 0.5; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }

  .bt-device {
    display: flex; align-items: center; gap: 10px;
    padding: 8px 10px; background: transparent; border: none; border-radius: var(--radius-md);
    color: var(--color-fg-shell); font-size: 0.8125rem; cursor: pointer;
    text-align: left; width: 100%; transition: background-color 0.1s ease;
  }
  .bt-device:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .bt-device.connected { background: color-mix(in srgb, var(--color-accent) 15%, transparent); border: 1px solid color-mix(in srgb, var(--color-accent) 30%, transparent); }
  .bt-device.connecting { opacity: 0.7; }
  .bt-device-icon { flex-shrink: 0; opacity: 0.7; }
  .bt-device-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .bt-device-name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; display: block; font-size: 0.8125rem; }
  .bt-device-detail { font-size: 0.6875rem; opacity: 0.5; display: flex; align-items: center; gap: 3px; }
  :global(.bt-battery-icon) { display: inline; vertical-align: middle; }

  .bt-scan-btn {
    display: flex; align-items: center; justify-content: center; gap: 6px;
    padding: 7px; background: transparent; border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-radius: var(--radius-md); color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    font-size: 0.75rem; cursor: pointer; transition: all 0.15s ease;
  }
  .bt-scan-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }

  :global(.spinning) { animation: spin 1s linear infinite; }
  @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
</style>
