<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Workspaces & Tiling settings page (Sprint B).
  ///
  /// Configures `compositor.toml [workspaces]` and `[layout]` via
  /// the generic `config_set` command (format-preserving via
  /// toml_writer, Sprint A). The compositor's TOML hot-reload picks
  /// changes up automatically — no daemon restart needed.

  import { onMount } from "svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { AddRemoveList } from "@arlen/ui-kit/components/ui/add-remove-list";
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
    if (m.app_id) parts.push(`app matches ${m.app_id}`);
    if (m.title) parts.push(`title matches ${m.title}`);
    if (m.window_type) parts.push(`type is ${m.window_type}`);
    if (parts.length === 0) parts.push("any window");
    const action = rule.action === "float" ? "floats" : "tiles";
    return `${parts.join(", ")}: ${action}`;
  }

  const LAYOUT_OPTIONS = [
    { value: "Horizontal", label: "Horizontal" },
    { value: "Vertical", label: "Vertical" },
  ];
</script>

<Page
  title={$t("s.ws.title")}
  description={$t("s.ws.desc")}
>
  <SectionGrid>
    <Section label={$t("s.ws.layout")}>
    <Row
      label={$t("s.ws.direction")}
      description={$t("s.ws.directionDesc")}
      id="workspace-layout"
    >
      {#snippet control()}
        <PopoverSelect
          value={workspaceLayout}
          options={LAYOUT_OPTIONS}
          ariaLabel={$t("s.ws.layoutAria")}
          onchange={setWorkspaceLayout}
        />
      {/snippet}
    </Row>
  </Section>

  <Section label={$t("s.ws.tiling")}>
    <Row
      label={$t("s.ws.innerGap")}
      description={$t("s.ws.innerGapDesc")}
      id="inner-gap"
    >
      {#snippet control()}
        <ValueSlider
          value={innerGap}
          min={0}
          max={32}
          step={1}
          unit="px"
          ariaLabel={$t("s.ws.innerGap")}
          onchange={setInnerGap}
        />
      {/snippet}
    </Row>

    <Row
      label={$t("s.ws.outerGap")}
      description={$t("s.ws.outerGapDesc")}
      id="outer-gap"
    >
      {#snippet control()}
        <ValueSlider
          value={outerGap}
          min={0}
          max={32}
          step={1}
          unit="px"
          ariaLabel={$t("s.ws.outerGap")}
          onchange={setOuterGap}
        />
      {/snippet}
    </Row>

    <Row
      label={$t("s.ws.smartGaps")}
      description={$t("s.ws.smartGapsDesc")}
      id="smart-gaps"
    >
      {#snippet control()}
        <Switch
          value={smartGaps}
          ariaLabel={$t("s.ws.smartGaps")}
          onchange={setSmartGaps}
        />
      {/snippet}
    </Row>

    <Row
      label={$t("s.ws.headers")}
      description={$t("s.ws.headersDesc")}
      id="tiled-headers"
    >
      {#snippet control()}
        <Switch
          value={tiledHeaders}
          ariaLabel={$t("s.ws.headers")}
          onchange={setTiledHeaders}
        />
      {/snippet}
    </Row>
  </Section>

  <Section label={$t("s.ws.rules")}>
    <Row
      label={$t("s.ws.perApp")}
      description={$t("s.ws.perAppDesc")}
      id="window-rules"
    >
      {#snippet control()}
        <span class="rule-count">
          {$t("s.ws.ruleCount", { count: windowRules.length })}
        </span>
      {/snippet}
    </Row>
    <div class="rules-list">
      <AddRemoveList
        items={windowRules}
        onremove={removeRule}
        onadd={() => (addRuleOpen = true)}
        addLabel="Add Rule"
        emptyMessage="No rules yet. Apps follow the global tiling default."
      >
        {#snippet itemSnippet({ item }: { item: WindowRule; index: number })}
          <code class="rule-code">{ruleSummary(item as WindowRule)}</code>
        {/snippet}
      </AddRemoveList>
    </div>
  </Section>
  </SectionGrid>
</Page>

<AddWindowRuleDialog
  open={addRuleOpen}
  onAdd={addRule}
  onCancel={() => (addRuleOpen = false)}
/>

<style>
  .rule-count {
    font-size: var(--text-sm);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }

  .rules-list {
    padding: 0 1rem 0.875rem;
  }

  .rule-code {
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: var(--text-xs);
  }
</style>
