<script lang="ts">
  /// Workspaces & Tiling settings page (Sprint B).
  ///
  /// Configures `compositor.toml [workspaces]` and `[layout]` via
  /// the generic `config_set` command (format-preserving via
  /// toml_writer, Sprint A). The compositor's TOML hot-reload picks
  /// changes up automatically — no daemon restart needed.

  import { onMount } from "svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import { Group } from "$lib/components/ui/group";
  import { Row } from "$lib/components/ui/row";
  import { Switch } from "$lib/components/ui/switch";
  import { ValueSlider } from "$lib/components/ui/value-slider";
  import { PopoverSelect } from "$lib/components/ui/popover-select";
  import { AddRemoveList } from "$lib/components/ui/add-remove-list";
  import AddWindowRuleDialog from "$lib/components/workspaces/AddWindowRuleDialog.svelte";
  import {
    compositor,
    LAYOUT_DEFAULTS,
    WORKSPACE_LAYOUT_DEFAULT,
    type WorkspaceLayout,
    type WindowRule,
  } from "$lib/stores/workspaces";

  onMount(() => {
    compositor.load();
  });

  // Reactive views into the loaded config. Defaults fill in when
  // a key isn't yet present in compositor.toml.
  const workspaceLayout = $derived<WorkspaceLayout>(
    ($compositor.data?.workspaces?.workspace_layout as WorkspaceLayout) ??
      WORKSPACE_LAYOUT_DEFAULT,
  );
  const innerGap = $derived<number>(
    $compositor.data?.layout?.inner_gap ?? LAYOUT_DEFAULTS.inner_gap,
  );
  const outerGap = $derived<number>(
    $compositor.data?.layout?.outer_gap ?? LAYOUT_DEFAULTS.outer_gap,
  );
  const smartGaps = $derived<boolean>(
    $compositor.data?.layout?.smart_gaps ?? LAYOUT_DEFAULTS.smart_gaps,
  );
  const tiledHeaders = $derived<boolean>(
    $compositor.data?.layout?.tiled_headers ?? LAYOUT_DEFAULTS.tiled_headers,
  );
  const windowRules = $derived<WindowRule[]>(
    ($compositor.data?.layout?.window_rules as WindowRule[]) ?? [],
  );

  let addRuleOpen = $state(false);

  async function setWorkspaceLayout(value: string) {
    await compositor.setValue("workspaces.workspace_layout", value);
  }

  async function setInnerGap(value: number) {
    await compositor.setValue("layout.inner_gap", value);
  }

  async function setOuterGap(value: number) {
    await compositor.setValue("layout.outer_gap", value);
  }

  async function setSmartGaps(value: boolean) {
    await compositor.setValue("layout.smart_gaps", value);
  }

  async function setTiledHeaders(value: boolean) {
    await compositor.setValue("layout.tiled_headers", value);
  }

  async function addRule(rule: WindowRule) {
    addRuleOpen = false;
    const next = [...windowRules, rule];
    await compositor.setValue("layout.window_rules", next);
  }

  async function removeRule(index: number) {
    const next = windowRules.filter((_, i) => i !== index);
    await compositor.setValue("layout.window_rules", next);
  }

  function ruleSummary(rule: WindowRule): string {
    const parts: string[] = [];
    const m = rule.match ?? {};
    if (m.app_id) parts.push(`app_id ~ /${m.app_id}/`);
    if (m.title) parts.push(`title ~ /${m.title}/`);
    if (m.window_type) parts.push(`type = ${m.window_type}`);
    if (parts.length === 0) parts.push("(empty matcher)");
    return `${parts.join(" · ")} → ${rule.action === "float" ? "Float" : "Tile"}`;
  }

  const LAYOUT_OPTIONS = [
    { value: "Horizontal", label: "Horizontal" },
    { value: "Vertical", label: "Vertical" },
  ];
</script>

<SettingsPage
  title="Workspaces & Tiling"
  description="Workspace layout direction, default tiling gaps, and per-app window rules."
>
  <Group label="Workspace Layout">
    <Row
      label="Layout direction"
      description="Vertical stacks workspaces top-to-bottom; Horizontal arranges them left-to-right."
      id="workspace-layout"
    >
      {#snippet control()}
        <PopoverSelect
          value={workspaceLayout}
          options={LAYOUT_OPTIONS}
          ariaLabel="Workspace layout"
          width="180px"
          onchange={setWorkspaceLayout}
        />
      {/snippet}
    </Row>
  </Group>

  <Group label="Tiling">
    <Row
      label="Inner gap"
      description="Pixels between adjacent tiled windows."
      id="inner-gap"
    >
      {#snippet control()}
        <ValueSlider
          value={innerGap}
          min={0}
          max={32}
          step={1}
          unit="px"
          ariaLabel="Inner gap"
          onchange={setInnerGap}
        />
      {/snippet}
    </Row>

    <Row
      label="Outer gap"
      description="Pixels between tiled windows and the screen edge."
      id="outer-gap"
    >
      {#snippet control()}
        <ValueSlider
          value={outerGap}
          min={0}
          max={32}
          step={1}
          unit="px"
          ariaLabel="Outer gap"
          onchange={setOuterGap}
        />
      {/snippet}
    </Row>

    <Row
      label="Smart gaps"
      description="Hide gaps when only one window is tiled on a workspace."
      id="smart-gaps"
    >
      {#snippet control()}
        <Switch
          value={smartGaps}
          ariaLabel="Smart gaps"
          onchange={setSmartGaps}
        />
      {/snippet}
    </Row>

    <Row
      label="Tiled window headers"
      description="Show window-control headers on single tiled windows. Off matches i3/sway/hyprland convention; stacks always keep their tab-bar header."
      id="tiled-headers"
    >
      {#snippet control()}
        <Switch
          value={tiledHeaders}
          ariaLabel="Tiled window headers"
          onchange={setTiledHeaders}
        />
      {/snippet}
    </Row>
  </Group>

  <Group label="Window Rules">
    <Row
      label="Per-app rules"
      description="Force specific apps to float or tile based on app_id, title, or window type. Patterns are regular expressions."
      id="window-rules"
    >
      {#snippet control()}
        <span class="rule-count">
          {windowRules.length}
          {windowRules.length === 1 ? "rule" : "rules"}
        </span>
      {/snippet}
    </Row>
    <div class="rules-list">
      <AddRemoveList
        items={windowRules}
        onremove={removeRule}
        onadd={() => (addRuleOpen = true)}
        addLabel="Add Rule"
        emptyMessage="No rules yet — apps follow the global tiling default."
      >
        {#snippet itemSnippet({ item })}
          <code class="rule-code">{ruleSummary(item as WindowRule)}</code>
        {/snippet}
      </AddRemoveList>
    </div>
  </Group>
</SettingsPage>

<AddWindowRuleDialog
  open={addRuleOpen}
  onAdd={addRule}
  onCancel={() => (addRuleOpen = false)}
/>

<style>
  .rule-count {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .rules-list {
    padding: 0 1rem 0.875rem;
  }

  .rule-code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 0.75rem;
  }
</style>
