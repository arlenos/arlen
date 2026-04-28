<script lang="ts">
  /// Per-monitor configuration panel.
  ///
  /// Renders the editable controls (Resolution, Refresh, Scale,
  /// Transform, VRR, Max BPC, Mirror, Primary, Enabled) for a single
  /// monitor. All edits flow into the bound `draft` and propagate
  /// upward via callbacks; the parent collects changes and decides
  /// when to call `applyConfig`.

  import type {
    Monitor,
    MonitorConfig,
    Transform,
    VrrState,
    EnabledKind,
    MonitorMode,
  } from "$lib/stores/displays";
  import { groupedResolutions } from "$lib/stores/displays";
  import { Switch } from "$lib/components/ui/switch";
  import { PopoverSelect } from "$lib/components/ui/popover-select";
  import { Button } from "$lib/components/ui/button";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";

  interface Props {
    monitor: Monitor;
    draft: MonitorConfig;
    /** Other monitors, used to populate the Mirror dropdown. */
    others: Monitor[];
    onChange: (draft: MonitorConfig) => void;
  }

  let { monitor, draft, others, onChange }: Props = $props();

  const resolutions = $derived(groupedResolutions(monitor.modes));
  const currentMode = $derived<MonitorMode | null>(
    draft.modeIndex !== null ? (monitor.modes[draft.modeIndex] ?? null) : null,
  );

  function pickResolution(width: number, height: number) {
    const matching = monitor.modes.findIndex(
      (m) => m.width === width && m.height === height,
    );
    if (matching >= 0) onChange({ ...draft, modeIndex: matching });
  }

  function pickRefresh(refreshMhz: number) {
    if (!currentMode) return;
    const idx = monitor.modes.findIndex(
      (m) =>
        m.width === currentMode.width &&
        m.height === currentMode.height &&
        m.refreshMhz === refreshMhz,
    );
    if (idx >= 0) onChange({ ...draft, modeIndex: idx });
  }

  function setTransform(t: Transform) {
    onChange({ ...draft, transform: t });
  }

  function setScale(scale: number) {
    onChange({ ...draft, scale });
  }

  function setEnabled(active: boolean) {
    onChange({
      ...draft,
      enabled: active ? { type: "active" } : { type: "disabled" },
    });
  }

  function setMirror(target: string) {
    if (target === "__none__") {
      onChange({ ...draft, enabled: { type: "active" } });
    } else {
      onChange({ ...draft, enabled: { type: "mirror", target } });
    }
  }

  function setVrr(vrr: VrrState) {
    onChange({ ...draft, vrr });
  }

  // Primary and Max BPC are part of the data model and persist via
  // displays.toml, but the wlr-output-management protocol cannot
  // change them at runtime (only the cosmic-output-management
  // extension can). D2 wires them through the cosmic extension; for
  // D1 we keep the toggle off the UI rather than ship a placebo.
  // See display-system.md §A5.

  const isActive = $derived(draft.enabled.type === "active");
  const isMirror = $derived(draft.enabled.type === "mirror");
  const mirrorTarget = $derived(
    draft.enabled.type === "mirror" ? draft.enabled.target : null,
  );

  const SCALE_PRESETS = [1.0, 1.25, 1.5, 1.75, 2.0];

  // Static option lists for the unified PopoverSelect dropdowns.
  // Resolution and refresh derive their options dynamically from the
  // current monitor; rotation and VRR are fixed enums.
  const TRANSFORM_OPTIONS = [
    { value: "normal", label: "Normal" },
    { value: "rotate-90", label: "90°" },
    { value: "rotate-180", label: "180°" },
    { value: "rotate-270", label: "270°" },
    { value: "flipped", label: "Flipped" },
    { value: "flipped-90", label: "Flipped 90°" },
    { value: "flipped-180", label: "Flipped 180°" },
    { value: "flipped-270", label: "Flipped 270°" },
  ];

  const VRR_OPTIONS = [
    { value: "enabled", label: "Enabled (auto)" },
    { value: "disabled", label: "Disabled" },
    { value: "force", label: "Force always-on" },
  ];

  const resolutionOptions = $derived(
    resolutions.map((r) => ({
      value: `${r.width}x${r.height}`,
      label: `${r.width} × ${r.height}`,
    })),
  );

  const refreshOptions = $derived(
    (
      resolutions.find(
        (r) => r.width === currentMode?.width && r.height === currentMode?.height,
      )?.refreshOptions ?? []
    ).map((opt) => ({
      value: String(opt.refreshMhz),
      label: `${(opt.refreshMhz / 1000).toFixed(2)} Hz`,
    })),
  );

  const mirrorOptions = $derived([
    { value: "__none__", label: "Independent" },
    ...others.map((o) => ({ value: o.connector, label: o.connector })),
  ]);
</script>

<SettingsRow label="Active" description="{monitor.make} {monitor.model}">
  {#snippet control()}
    <Switch value={isActive} onchange={setEnabled} />
  {/snippet}
</SettingsRow>

{#if isActive}
  <SettingsRow label="Resolution">
    {#snippet control()}
      <PopoverSelect
        value={currentMode ? `${currentMode.width}x${currentMode.height}` : ""}
        options={resolutionOptions}
        ariaLabel="Resolution"
        width="180px"
        onchange={(v) => {
          const [w, h] = v.split("x").map(Number);
          if (w && h) pickResolution(w, h);
        }}
      />
    {/snippet}
  </SettingsRow>

  <SettingsRow label="Refresh Rate">
    {#snippet control()}
      <PopoverSelect
        value={currentMode ? String(currentMode.refreshMhz) : ""}
        options={refreshOptions}
        ariaLabel="Refresh rate"
        width="180px"
        onchange={(v) => pickRefresh(Number(v))}
      />
    {/snippet}
  </SettingsRow>

  <SettingsRow label="Scale">
    {#snippet control()}
      <div class="presets">
        {#each SCALE_PRESETS as p}
          {@const isSelected = Math.abs(draft.scale - p) < 0.01}
          <Button
            variant={isSelected ? "secondary" : "outline"}
            size="sm"
            onclick={() => setScale(p)}
          >
            {p}×
          </Button>
        {/each}
      </div>
    {/snippet}
  </SettingsRow>

  <SettingsRow label="Rotation">
    {#snippet control()}
      <PopoverSelect
        value={draft.transform}
        options={TRANSFORM_OPTIONS}
        ariaLabel="Rotation"
        width="180px"
        onchange={(v) => setTransform(v as Transform)}
      />
    {/snippet}
  </SettingsRow>

  <SettingsRow
    label="Adaptive Sync"
    description="Variable refresh rate (VRR / FreeSync)"
  >
    {#snippet control()}
      <PopoverSelect
        value={draft.vrr}
        options={VRR_OPTIONS}
        ariaLabel="Adaptive sync"
        width="180px"
        onchange={(v) => setVrr(v as VrrState)}
      />
    {/snippet}
  </SettingsRow>

  {#if others.length > 0}
    <SettingsRow label="Mirror To">
      {#snippet control()}
        <PopoverSelect
          value={mirrorTarget ?? "__none__"}
          options={mirrorOptions}
          ariaLabel="Mirror target"
          width="180px"
          onchange={setMirror}
        />
      {/snippet}
    </SettingsRow>
  {/if}
{:else}
  <div class="hint-row">
    This output is currently disabled. Toggle <em>Active</em> to wake it.
  </div>
{/if}

<style>
  .presets {
    display: flex;
    gap: 4px;
  }

  .hint-row {
    padding: 12px 16px;
    font-size: 0.85rem;
    color: color-mix(in srgb, var(--color-fg-app) 55%, transparent);
  }
</style>
