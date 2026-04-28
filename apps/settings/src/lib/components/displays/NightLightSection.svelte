<script lang="ts">
  /// Night light section for the Display panel.
  ///
  /// Drives the compositor's gamma engine end-to-end: the user
  /// changes a control, we invoke a Tauri command in desktop-shell
  /// (`night_light_set` / `night_light_set_schedule` /
  /// `night_light_set_location`) which persists to shell.toml AND
  /// dispatches the matching `lunaris-shell-overlay` request. The
  /// compositor warms the screen within ~200ms.
  ///
  /// We read the persisted state on mount and reflect later writes
  /// from the compositor / shell via the existing
  /// `lunaris://shell-config-changed` event the shell emits when
  /// shell.toml is rewritten.

  import { onMount, onDestroy } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { Switch } from "$lib/components/ui/switch";
  import { ValueSlider } from "$lib/components/ui/value-slider";
  import { PopoverSelect } from "$lib/components/ui/popover-select";
  import { TimeInput } from "$lib/components/ui/time-input";
  import { NumberInput } from "$lib/components/ui/number-input";
  import SettingsGroup from "$lib/components/settings/SettingsGroup.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";

  // Mirrors `app-settings/src-tauri/src/commands/night_light.rs::NightLightState`.
  // Kept in hand-sync; if the schema grows, ts-rs would be the right
  // fix but for D2 the surface is small.
  type ScheduleMode = "manual" | "sunset_sunrise" | "custom";

  interface NightLightConfig {
    enabled: boolean;
    temperature: number;
    schedule: ScheduleMode;
    custom_start: number;
    custom_end: number;
    latitude: number;
    longitude: number;
  }

  const DEFAULT_CONFIG: NightLightConfig = {
    enabled: false,
    temperature: 3400,
    schedule: "manual",
    custom_start: 22 * 60,
    custom_end: 7 * 60,
    latitude: 0,
    longitude: 0,
  };

  let cfg = $state<NightLightConfig>({ ...DEFAULT_CONFIG });
  let unlistenChanged: UnlistenFn | null = null;

  const SCHEDULE_OPTIONS = [
    { value: "manual", label: "Off (manual toggle only)" },
    { value: "sunset_sunrise", label: "Sunset to sunrise" },
    { value: "custom", label: "Custom hours" },
  ];

  async function reloadFromDisk() {
    try {
      const initial = await invoke<NightLightConfig>("night_light_get_state");
      cfg = { ...DEFAULT_CONFIG, ...initial };
    } catch (err) {
      console.warn("night_light: read shell.toml failed:", err);
    }
  }

  onMount(async () => {
    await reloadFromDisk();
    // Watch the shared shell.toml for external writes (e.g. the
    // QuickSettings toggle in desktop-shell). Re-read instead of
    // patching because the watcher event payload is empty.
    unlistenChanged = await listen("lunaris://shell-config-changed", () => {
      reloadFromDisk();
    });
  });

  onDestroy(() => {
    unlistenChanged?.();
  });

  function setEnabled(enabled: boolean) {
    cfg = { ...cfg, enabled };
    invoke("night_light_set", {
      enabled,
      temperature: cfg.temperature,
    }).catch((err) => console.warn("night_light_set failed:", err));
  }

  // Slider drag fires onchange a lot. Debounce so we don't flood the
  // compositor with redundant gamma writes. 80ms keeps the live
  // preview snappy without being chatty.
  let tempDebounce: ReturnType<typeof setTimeout> | null = null;
  function setTemperature(t: number) {
    cfg = { ...cfg, temperature: t };
    if (tempDebounce) clearTimeout(tempDebounce);
    tempDebounce = setTimeout(() => {
      invoke("night_light_set", {
        enabled: cfg.enabled,
        temperature: t,
      }).catch((err) => console.warn("night_light_set failed:", err));
    }, 80);
  }

  function setSchedule(mode: string) {
    const m = mode as ScheduleMode;
    cfg = { ...cfg, schedule: m };
    invoke("night_light_set_schedule", {
      schedule: m,
      customStart: cfg.custom_start,
      customEnd: cfg.custom_end,
    }).catch((err) => console.warn("night_light_set_schedule failed:", err));
  }

  function setCustomStart(hhmm: string) {
    const minutes = parseHhmm(hhmm);
    cfg = { ...cfg, custom_start: minutes };
    invoke("night_light_set_schedule", {
      schedule: cfg.schedule,
      customStart: minutes,
      customEnd: cfg.custom_end,
    }).catch((err) => console.warn("night_light_set_schedule failed:", err));
  }

  function setCustomEnd(hhmm: string) {
    const minutes = parseHhmm(hhmm);
    cfg = { ...cfg, custom_end: minutes };
    invoke("night_light_set_schedule", {
      schedule: cfg.schedule,
      customStart: cfg.custom_start,
      customEnd: minutes,
    }).catch((err) => console.warn("night_light_set_schedule failed:", err));
  }

  function setLatitude(lat: number) {
    cfg = { ...cfg, latitude: lat };
    invoke("night_light_set_location", {
      latitude: lat,
      longitude: cfg.longitude,
    }).catch((err) => console.warn("night_light_set_location failed:", err));
  }

  function setLongitude(lon: number) {
    cfg = { ...cfg, longitude: lon };
    invoke("night_light_set_location", {
      latitude: cfg.latitude,
      longitude: lon,
    }).catch((err) => console.warn("night_light_set_location failed:", err));
  }

  function parseHhmm(hhmm: string): number {
    const [h, m] = hhmm.split(":").map((s) => parseInt(s, 10));
    if (Number.isNaN(h) || Number.isNaN(m)) return 0;
    return (h * 60 + m) % (24 * 60);
  }

  function formatHhmm(minutes: number): string {
    const m = ((minutes % (24 * 60)) + 24 * 60) % (24 * 60);
    const h = Math.floor(m / 60);
    const mm = m % 60;
    return `${String(h).padStart(2, "0")}:${String(mm).padStart(2, "0")}`;
  }

  const locationUnset = $derived(cfg.latitude === 0 && cfg.longitude === 0);
</script>

<SettingsGroup label="Night Light">
  <SettingsRow
    label="Active"
    description="Warm the screen to reduce blue light."
  >
    {#snippet control()}
      <Switch value={cfg.enabled} onchange={setEnabled} />
    {/snippet}
  </SettingsRow>

  <SettingsRow
    label="Color Temperature"
    description="Lower is warmer (more orange)."
  >
    {#snippet control()}
      <div class="slider-cell">
        <ValueSlider
          value={cfg.temperature}
          min={1000}
          max={6500}
          step={100}
          unit="K"
          ariaLabel="Color temperature"
          onchange={setTemperature}
        />
      </div>
    {/snippet}
  </SettingsRow>

  <SettingsRow label="Schedule">
    {#snippet control()}
      <PopoverSelect
        value={cfg.schedule}
        options={SCHEDULE_OPTIONS}
        ariaLabel="Night light schedule"
        width="220px"
        onchange={setSchedule}
      />
    {/snippet}
  </SettingsRow>

  {#if cfg.schedule === "custom"}
    <SettingsRow label="Start">
      {#snippet control()}
        <TimeInput
          value={formatHhmm(cfg.custom_start)}
          ariaLabel="Custom start time"
          onchange={setCustomStart}
        />
      {/snippet}
    </SettingsRow>
    <SettingsRow label="End">
      {#snippet control()}
        <TimeInput
          value={formatHhmm(cfg.custom_end)}
          ariaLabel="Custom end time"
          onchange={setCustomEnd}
        />
      {/snippet}
    </SettingsRow>
  {/if}

  {#if cfg.schedule === "sunset_sunrise"}
    <SettingsRow
      label="Latitude"
      description={locationUnset
        ? "Set both coordinates to enable sunset/sunrise mode."
        : undefined}
    >
      {#snippet control()}
        <NumberInput
          value={cfg.latitude}
          min={-90}
          max={90}
          step={0.0001}
          ariaLabel="Latitude"
          width="160px"
          onchange={setLatitude}
        />
      {/snippet}
    </SettingsRow>
    <SettingsRow label="Longitude">
      {#snippet control()}
        <NumberInput
          value={cfg.longitude}
          min={-180}
          max={180}
          step={0.0001}
          ariaLabel="Longitude"
          width="160px"
          onchange={setLongitude}
        />
      {/snippet}
    </SettingsRow>
  {/if}
</SettingsGroup>

<style>
  /* Slider needs more horizontal room than other controls. SettingsRow
     gives the control area a hard right-aligned width; the wrapper
     here lets the slider fill 220px instead of collapsing. */
  .slider-cell {
    width: 220px;
  }

</style>
