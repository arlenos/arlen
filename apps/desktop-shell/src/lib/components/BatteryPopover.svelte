<script lang="ts">
  /// Battery popover: status + power profiles.

  import { activePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Separator } from "@arlen/ui-kit/components/ui/separator/index.js";
  import * as Tooltip from "@arlen/ui-kit/components/ui/tooltip";
  import { Zap, Battery, Leaf, Scale } from "lucide-svelte";
  import ShellPopover from "$lib/components/shared/ShellPopover.svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";

  interface BatteryStatus {
    percentage: number;
    charging: boolean;
    time_remaining_minutes: number | null;
  }

  let status = $state<BatteryStatus | null>(null);
  let powerProfile = $state("balanced");
  /// True once the first poll answered — before that the status area
  /// stays blank instead of claiming "No battery" while loading.
  let polled = $state(false);

  async function poll() {
    try { status = await invoke<BatteryStatus | null>("get_battery_status"); } catch {}
    try { powerProfile = await invoke<string>("get_power_profile"); } catch {}
    polled = true;
  }

  $effect(() => {
    if ($activePopover === "battery") poll();
  });

  async function setProfile(p: string) {
    // Optimistic UI update so the profile pill reflects the click
    // immediately. Re-poll afterwards so the `time_remaining` estimate
    // reflects the new profile (UPower recalculates based on current
    // drain; previously this stayed stale until the next upstream
    // battery event).
    powerProfile = p;
    try {
      await invoke("set_power_profile", { profile: p });
    } catch (e) {
      console.warn("[battery] set_power_profile failed:", e);
    }
    await poll();
  }

  function timeStr(mins: number | null): string {
    if (!mins || mins <= 0) return "";
    const h = Math.floor(mins / 60);
    const m = mins % 60;
    return h > 0 ? `${h}h ${m}min` : `${m}min`;
  }

  const PROFILES: { id: string; label: string; icon: typeof Leaf }[] = [
    { id: "power-saver", label: "Power Saver", icon: Leaf },
    { id: "balanced", label: "Balanced", icon: Scale },
    { id: "performance", label: "Performance", icon: Zap },
  ];
</script>

<ShellPopover id="battery" width={240} right={50} bodyPadding="12px" bodyGap="8px">
  {#snippet header()}
    <PopoverHeader icon={Battery} title="Power" />
  {/snippet}

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
  {:else if polled}
    <div class="bat-status">
      <span class="bat-detail">No battery found</span>
    </div>
  {/if}

  <Separator class="opacity-10" />

  <div class="bat-section">
    <span class="bat-heading">Power Mode</span>
    <div class="bat-profiles">
      {#each PROFILES as p (p.id)}
        <Tooltip.Root>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <button
                {...props}
                class="bat-pill"
                class:active={powerProfile === p.id}
                aria-label={p.label}
                onclick={(e) => { e.stopPropagation(); setProfile(p.id); }}
              >
                <p.icon size={14} strokeWidth={1.5} />
              </button>
            {/snippet}
          </Tooltip.Trigger>
          <Tooltip.TooltipContent side="bottom">
            {p.label}
          </Tooltip.TooltipContent>
        </Tooltip.Root>
      {/each}
    </div>
  </div>
</ShellPopover>

<style>
  .bat-status { display: flex; flex-direction: column; gap: 2px; }
  .bat-pct { font-size: var(--text-xl); font-weight: 600; }
  .bat-detail { font-size: var(--text-2xs); opacity: 0.5; display: flex; align-items: center; gap: 4px; }

  .bat-section { display: flex; flex-direction: column; gap: 8px; }
  .bat-heading { font-size: var(--text-2xs); font-weight: 600; opacity: 0.5; }
  .bat-profiles { display: flex; gap: 4px; }

  .bat-pill {
    flex: 1; height: 32px;
    display: flex; align-items: center; justify-content: center;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-radius: var(--radius-input); background: transparent;
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
    padding: 0;
    transition:
      background-color var(--duration-fast, 150ms) ease,
      border-color var(--duration-fast, 150ms) ease,
      color var(--duration-fast, 150ms) ease;
  }
  .bat-pill:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }
  .bat-pill.active {
    background: color-mix(in srgb, var(--color-accent) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-accent) 30%, transparent);
    color: var(--color-fg-shell);
  }
</style>
