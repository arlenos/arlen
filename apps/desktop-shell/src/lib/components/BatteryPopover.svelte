<script lang="ts">
  import { t } from "$lib/i18n/messages";
  import { durationText } from "$lib/duration";
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
  /// `null` until a read answers. It used to default to "balanced", which is a
  /// claim about the machine's power mode made before anyone had asked - and the
  /// pill for it rendered as selected.
  let powerProfile = $state<string | null>(null);
  /// True once the first poll answered — before that the status area
  /// stays blank instead of claiming "No battery" while loading.
  let polled = $state(false);
  /// The read failed, which is a third thing from "answered" and "not yet". The
  /// `polled` flag above already separated loading from answered; without this,
  /// a refused read set `polled` and the panel said "No battery" - a claim about
  /// the hardware from a question nobody got an answer to.
  let unreadable = $state(false);

  /// The mode was pressed and the machine said no.
  ///
  /// The press is optimistic and the poll that follows re-reads the truth, so
  /// the pill flips back on its own - which is a signal without being an
  /// explanation, and looks identical to a mis-click. The refusal used to reach
  /// `console.warn` alone, and this webview has no console anybody reads.
  ///
  /// It names what still holds rather than only what failed, because the pill
  /// the poll restored is the answer to the question the person is now asking.
  let profileRefused = $state(false);

  async function poll() {
    unreadable = false;
    try {
      status = await invoke<BatteryStatus | null>("get_battery_status");
    } catch {
      status = null;
      unreadable = true;
    }
    try {
      powerProfile = await invoke<string>("get_power_profile");
    } catch {
      powerProfile = null;
      unreadable = true;
    }
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
    profileRefused = false;
    try {
      await invoke("set_power_profile", { profile: p });
    } catch (e) {
      console.warn("[battery] set_power_profile failed:", e);
      profileRefused = true;
    }
    await poll();
  }


  // `label` is a message KEY, resolved with $t where it renders: a top-level
  // const captures the locale at import and would never follow a switch.
  const PROFILES: { id: string; label: string; icon: typeof Leaf }[] = [
    { id: "power-saver", label: "sh.bat.powerSaver", icon: Leaf },
    { id: "balanced", label: "sh.bat.balanced", icon: Scale },
    { id: "performance", label: "sh.bat.performance", icon: Zap },
  ];
</script>

<ShellPopover id="battery" width={240} right={50} bodyPadding="12px" bodyGap="8px">
  {#snippet header()}
    <PopoverHeader icon={Battery} title={$t("sh.bat.title")} />
  {/snippet}

  {#if status}
    <div class="bat-status">
      <span class="bat-pct">{status.percentage}%</span>
      <span class="bat-detail">
        {#if status.charging}
          <Zap size={12} strokeWidth={2} />{status.time_remaining_minutes
            ? $t("sh.bat.chargingEta", { time: durationText($t, status.time_remaining_minutes) })
            : $t("sh.bat.charging")}
        {:else if status.time_remaining_minutes}
          {$t("sh.bat.remaining", { time: durationText($t, status.time_remaining_minutes) })}
        {:else}
          {$t("sh.bat.onBattery")}
        {/if}
      </span>
    </div>
  {:else if unreadable}
    <div class="bat-status">
      <span class="bat-detail">{$t("sh.bat.stateUnknown")}</span>
    </div>
  {:else if polled}
    <div class="bat-status">
      <span class="bat-detail">{$t("sh.bat.noBattery")}</span>
    </div>
  {/if}

  <Separator class="opacity-10" />

  <div class="bat-section">
    <span class="bat-heading">{$t("sh.bat.powerMode")}</span>
    {#if profileRefused}
      <p class="bat-refused" role="alert">{$t("sh.bat.profileRefused")}</p>
    {/if}
    <div class="bat-profiles">
      {#each PROFILES as p (p.id)}
        <Tooltip.Root>
          <Tooltip.Trigger>
            {#snippet child({ props })}
              <button
                {...props}
                class="bat-pill"
                class:active={powerProfile === p.id}
                aria-label={$t(p.label)}
                onclick={(e) => { e.stopPropagation(); setProfile(p.id); }}
              >
                <p.icon size={14} strokeWidth={1.5} />
              </button>
            {/snippet}
          </Tooltip.Trigger>
          <Tooltip.TooltipContent side="bottom">
            {$t(p.label)}
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
  .bat-refused {
    margin: 0 0 0.35rem;
    font-size: 0.75rem;
    line-height: 1.35;
    color: var(--color-error, #f87171);
  }

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
