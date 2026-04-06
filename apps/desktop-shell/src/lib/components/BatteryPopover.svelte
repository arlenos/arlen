<script lang="ts">
  /// Battery popover: status + power profiles.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { Zap, Battery, Leaf, Scale } from "lucide-svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";

  interface BatteryStatus {
    percentage: number;
    charging: boolean;
    time_remaining_minutes: number | null;
  }

  let status = $state<BatteryStatus | null>(null);
  let powerProfile = $state("balanced");

  async function poll() {
    try { status = await invoke<BatteryStatus | null>("get_battery_status"); } catch {}
    try { powerProfile = await invoke<string>("get_power_profile"); } catch {}
  }

  $effect(() => {
    if ($activePopover === "battery") poll();
  });

  function setProfile(p: string) {
    powerProfile = p;
    invoke("set_power_profile", { profile: p }).catch(() => {});
  }

  function timeStr(mins: number | null): string {
    if (!mins || mins <= 0) return "";
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return h > 0 ? `${h}h ${m}min` : `${m}min`;
  }
</script>

{#if $activePopover === "battery"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-backdrop" onclick={closePopover}></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-panel pop-battery shell-popover" onclick={(e) => e.stopPropagation()}>
    <PopoverHeader icon={Battery} title="Power" />
    <div class="pop-body">
    {#if status}
      <div class="bat-status">
        <span class="bat-pct">{status.percentage}%</span>
        <span class="bat-detail">
          {#if status.charging}
            <Zap size={12} strokeWidth={2} />Charging{#if status.time_remaining_minutes} ({timeStr(status.time_remaining_minutes)}){/if}
          {:else if status.time_remaining_minutes}
            {timeStr(status.time_remaining_minutes)} remaining
          {:else}
            On battery
          {/if}
        </span>
      </div>
    {:else}
      <div class="bat-status">
        <span class="bat-detail">No battery detected</span>
      </div>
    {/if}

    <Separator class="opacity-10" />

    <div class="bat-section">
      <span class="bat-heading">Power Mode</span>
      <div class="bat-profiles">
        <button class="bat-pill" class:active={powerProfile === "power-saver"}
          onclick={(e) => { e.stopPropagation(); setProfile("power-saver"); }}
          title="Power Saver">
          <Leaf size={14} strokeWidth={1.5} />
        </button>
        <button class="bat-pill" class:active={powerProfile === "balanced"}
          onclick={(e) => { e.stopPropagation(); setProfile("balanced"); }}
          title="Balanced">
          <Scale size={14} strokeWidth={1.5} />
        </button>
        <button class="bat-pill" class:active={powerProfile === "performance"}
          onclick={(e) => { e.stopPropagation(); setProfile("performance"); }}
          title="Performance">
          <Zap size={14} strokeWidth={1.5} />
        </button>
      </div>
    </div>
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
    color: var(--color-fg-shell); padding: 12px;
    display: flex; flex-direction: column; gap: 8px;
    animation: pop-open 100ms ease-out both;
  }
  .pop-battery { right: 50px; width: 240px; padding: 0; }
  .pop-body { padding: 12px; display: flex; flex-direction: column; gap: 8px; }
  @keyframes pop-open { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }

  .bat-status { display: flex; flex-direction: column; gap: 2px; }
  .bat-pct { font-size: 1.25rem; font-weight: 600; }
  .bat-detail { font-size: 0.6875rem; opacity: 0.5; display: flex; align-items: center; gap: 4px; }

  .bat-section { display: flex; flex-direction: column; gap: 8px; }
  .bat-heading { font-size: 0.6875rem; font-weight: 600; opacity: 0.5; }
  .bat-profiles { display: flex; gap: 4px; }

  .bat-pill {
    flex: 1; height: 32px;
    display: flex; align-items: center; justify-content: center;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-radius: 6px; background: transparent;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    cursor: pointer; padding: 0;
    transition: all 0.15s ease;
  }
  .bat-pill:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }
  .bat-pill.active {
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-fg-shell) 30%, transparent);
    color: var(--color-fg-shell);
  }
</style>
