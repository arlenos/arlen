<script lang="ts">
  /// Bluetooth popover: device list with context menus, scan, power toggle.

  import { activePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { Separator } from "@arlen/ui-kit/components/ui/separator/index.js";
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu/index.js";
  import ShellPopover from "$lib/components/shared/ShellPopover.svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import PopoverErrorBanner from "$lib/components/shared/PopoverErrorBanner.svelte";
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

  async function load() {
    loading = true;
    error = null;
    try {
      btState = await invoke<BluetoothState>("get_bluetooth_state");
    } catch {
      error = "Could not load Bluetooth";
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
    return () => {
      unlisten.then((fn) => fn()).catch(() => {});
      // Hard cleanup on component destroy: the $effect above handles
      // the scan-timer on popover close, but an abrupt unmount (window
      // hide, HMR full-reload) bypasses it and would leave the 10-s
      // timer scheduled on a detached closure.
      if (scanTimer) {
        clearTimeout(scanTimer);
        scanTimer = null;
      }
      if (btState?.discovering) {
        invoke("stop_bluetooth_scan").catch(() => {});
      }
    };
  });

  async function togglePower() {
    if (!btState) return;
    try {
      await invoke("set_bluetooth_powered", { enabled: !btState.powered });
      await load();
    } catch {
      error = "Could not turn Bluetooth on or off";
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
      catch { error = "Could not disconnect"; }
      await load();
      return;
    }
    connectingTo = dev.path;
    try {
      if (!dev.paired) await invoke("pair_bluetooth_device", { path: dev.path });
      await invoke("connect_bluetooth_device", { path: dev.path });
    } catch {
      error = "Could not connect";
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
    try { await invoke("connect_bluetooth_device", { path }); } catch { error = "Could not connect"; }
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
                <span
                  class="bt-battery"
                  class:bt-battery-spaced={connectingTo === dev.path ||
                    dev.connected ||
                    dev.paired}
                >
                  <BatteryMedium size={12} strokeWidth={1.5} class="bt-battery-icon" />
                  {dev.battery_percentage}%
                </span>
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
        <ContextMenu.Item onclick={() => remove(dev.path)} class="text-[var(--color-error)]">
          <Trash2 size={14} class="mr-2" />Forget Device
        </ContextMenu.Item>
      {/if}
    </ContextMenu.Content>
  </ContextMenu.Root>
{/snippet}

<ShellPopover id="bluetooth" width={280} right={80} bodyPadding="12px" bodyGap="6px">
  {#snippet header()}
    <PopoverHeader
      icon={Bluetooth}
      title="Bluetooth"
      toggled={btState?.powered ?? false}
      onToggle={togglePower}
    />
  {/snippet}

  {#if !btState}
    {#if error}
      <PopoverErrorBanner message={error} />
    {:else}
      <div class="bt-msg">
        <Bluetooth size={32} strokeWidth={1} />
        <span>Loading...</span>
      </div>
    {/if}
  {:else if !btState.available}
    <div class="bt-msg">
      <BluetoothOff size={32} strokeWidth={1} />
      <span>Bluetooth is not available on this device</span>
    </div>
  {:else if !btState.powered}
    <div class="bt-msg">
      <BluetoothOff size={32} strokeWidth={1} />
      <span>Bluetooth is off</span>
      <span class="bt-hint">Turn Bluetooth back on with the switch above</span>
    </div>
  {:else}
    {#if error}
      <PopoverErrorBanner message={error} />
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
</ShellPopover>

<style>
  .bt-msg { display: flex; flex-direction: column; align-items: center; gap: 8px; padding: 24px 12px; color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent); text-align: center; font-size: 0.8125rem; }
  .bt-hint { font-size: 0.6875rem; opacity: 0.5; }

  .bt-section-label { font-size: 0.6875rem; opacity: 0.5; font-weight: 600; text-transform: uppercase; letter-spacing: 0.04em; }

  .bt-device {
    display: flex; align-items: center; gap: 10px;
    padding: 8px 10px; background: transparent; border: none; border-radius: var(--radius-input);
    color: var(--color-fg-shell); font-size: 0.8125rem;
    text-align: left; width: 100%;
    transition: background-color var(--duration-micro, 100ms) ease;
  }
  .bt-device:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); }
  .bt-device.connected { background: color-mix(in srgb, var(--color-accent) 15%, transparent); border: 1px solid color-mix(in srgb, var(--color-accent) 30%, transparent); }
  .bt-device.connecting { opacity: 0.7; }
  .bt-device-icon { flex-shrink: 0; opacity: 0.7; }
  .bt-device-info { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; }
  .bt-device-name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; display: block; font-size: 0.8125rem; }
  .bt-device-detail { font-size: 0.6875rem; opacity: 0.5; display: flex; align-items: center; gap: 3px; }
  /* Battery reading sits apart from the status word by spacing, not
     a separator glyph. */
  .bt-battery { display: inline-flex; align-items: center; gap: 3px; }
  .bt-battery-spaced { margin-left: 6px; }
  :global(.bt-battery-icon) { display: inline; vertical-align: middle; }

  .bt-scan-btn {
    display: flex; align-items: center; justify-content: center; gap: 6px;
    padding: 7px; background: transparent; border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-radius: var(--radius-input); color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    font-size: 0.75rem;
    transition:
      background-color var(--duration-fast, 150ms) ease,
      color var(--duration-fast, 150ms) ease;
  }
  .bt-scan-btn:hover { background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent); color: var(--color-fg-shell); }

  :global(.spinning) { animation: spin 1s linear infinite; }
  @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
</style>
