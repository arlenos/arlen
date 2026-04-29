<script lang="ts">
  /// Brightness section for the Display panel.
  ///
  /// Routes through `app-settings`'s `brightness_set` command,
  /// which uses logind D-Bus the same way QuickSettings does — so
  /// dragging the slider here updates the panel and the
  /// QuickSettings slider reflects it on next read.
  ///
  /// Slider math: the backend exposes a `[0.0, 1.0]` fraction with
  /// `^2.2` gamma already applied, so we work in slider-space here
  /// directly. No need to know the device's raw range.

  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { ValueSlider } from "$lib/components/ui/value-slider";
  import { PopoverSelect } from "$lib/components/ui/popover-select";
  import SettingsGroup from "$lib/components/settings/SettingsGroup.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";

  interface BacklightDevice {
    name: string;
    kind: string;
    max: number;
    current: number;
  }

  interface BrightnessSnapshot {
    device: BacklightDevice;
    fraction: number;
  }

  let snapshots = $state<BrightnessSnapshot[]>([]);
  let selected = $state<string | null>(null);
  let percent = $state<number>(100);

  // Hardware writes are coalesced to ~30 Hz so a fast drag doesn't
  // flood logind with D-Bus calls. The persisted shell.toml is
  // updated via the desktop-shell side; this section is a remote
  // for the same hardware so we don't write to that file from
  // app-settings.
  let applyTimer: ReturnType<typeof setTimeout> | null = null;

  async function reload() {
    try {
      snapshots = await invoke<BrightnessSnapshot[]>("brightness_get_devices");
    } catch (err) {
      console.warn("brightness_get_devices failed:", err);
      snapshots = [];
    }
    if (snapshots.length > 0 && !selected) {
      selected = snapshots[0]!.device.name;
    }
    syncFromSelected();
  }

  function syncFromSelected() {
    const cur = snapshots.find((s) => s.device.name === selected);
    if (cur) {
      percent = Math.round(cur.fraction * 100);
    }
  }

  // The hardware Fn-row keys go through the desktop-shell process,
  // not through ours — Tauri events don't cross processes. Best
  // we can do without a separate IPC channel is re-read sysfs on
  // every focus change so the slider stays close to the actual
  // hardware state when the user comes back to Settings.
  function onVisibility() {
    if (typeof document !== "undefined" && !document.hidden) {
      reload();
    }
  }

  onMount(() => {
    reload();
    document.addEventListener("visibilitychange", onVisibility);
  });

  onDestroy(() => {
    if (typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", onVisibility);
    }
  });

  function setPercent(p: number) {
    percent = p;
    if (!selected) return;
    if (applyTimer) clearTimeout(applyTimer);
    const dev = selected;
    const fraction = p / 100;
    applyTimer = setTimeout(() => {
      invoke("brightness_set", { device: dev, value: fraction }).catch(
        (err) => console.warn("brightness_set failed:", err),
      );
    }, 32);
  }

  function selectDevice(name: string) {
    selected = name;
    syncFromSelected();
  }

  const deviceOptions = $derived(
    snapshots.map((s) => ({
      value: s.device.name,
      label: `${s.device.name} (${s.device.kind})`,
    })),
  );
</script>

<SettingsGroup label="Brightness">
  {#if snapshots.length === 0}
    <div class="empty">
      No backlight-controllable display detected. External monitors
      (HDMI / DisplayPort) typically don't expose software
      brightness; use the monitor's hardware buttons instead.
    </div>
  {:else}
    {#if snapshots.length > 1}
      <SettingsRow
        label="Display"
        description="Pick which panel the slider controls."
      >
        {#snippet control()}
          <PopoverSelect
            value={selected ?? ""}
            options={deviceOptions}
            ariaLabel="Brightness device"
            width="220px"
            onchange={selectDevice}
          />
        {/snippet}
      </SettingsRow>
    {/if}

    <SettingsRow
      label="Brightness"
      description="Lower for dim rooms, higher for daylight."
    >
      {#snippet control()}
        <div class="slider-cell">
          <ValueSlider
            value={percent}
            min={0}
            max={100}
            step={1}
            unit="%"
            ariaLabel="Brightness"
            onchange={setPercent}
          />
        </div>
      {/snippet}
    </SettingsRow>
  {/if}
</SettingsGroup>

<style>
  .slider-cell {
    width: 220px;
  }

  .empty {
    padding: 16px;
    font-size: 0.85rem;
    color: color-mix(in srgb, var(--color-fg-app) 55%, transparent);
  }
</style>
