<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Geometry: the theme's shape. A master Roundness knob up front; the per-radius
  /// bases, window corners, spacing and tiling gaps follow, with the granular
  /// per-token overrides behind expanders. Same two-column split, override-row and
  /// live preview as the Colours page (the preview corners round live as you drag).
  /// Rich by structure, not omission (appearance-surface.md).
  ///
  /// Mock-vs-live: reads a fixture; the per-radius / window-corner / spacing
  /// overrides need the theme.toml override backend (flagged for the coder).
  import { ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import OverrideRow from "$lib/components/appearance/OverrideRow.svelte";
  import ThemePreview from "$lib/components/appearance/ThemePreview.svelte";
  import { effective as colorsEffective } from "$lib/stores/themeColors";
  import {
    GEOM_FIELDS,
    overrides,
    effective,
    smartGaps,
    smartGapsOverridden,
    isOverridden,
    setGeom,
    resetGeom,
    previewRadiusVars,
    type GeomField,
  } from "$lib/stores/themeGeometry";

  const field = (key: string) => GEOM_FIELDS.find((f) => f.key === key)!;
  const inGroup = (g: GeomField["group"], tier: GeomField["tier"]) =>
    GEOM_FIELDS.filter((f) => f.group === g && f.tier === tier);

  const roundnessFull = inGroup("roundness", "full");
  const spacingFull = inGroup("spacing", "full");

  function displayVal(f: GeomField, eff: Record<string, number>): number {
    return Math.round(eff[f.key] * (f.scale ?? 1));
  }
  function onSlide(f: GeomField, v: number) {
    setGeom(f.key, v / (f.scale ?? 1));
  }

  function toggleSmartGaps(v: boolean) {
    smartGaps.set(v);
    smartGapsOverridden.set(v !== true);
  }
  function resetSmartGaps() {
    smartGaps.set(true);
    smartGapsOverridden.set(false);
  }
</script>

<Page
  title={$t("s.geom.title")}
  description={$t("s.geom.desc")}
>
  <SectionGrid>
    <div class="editor span-full">
    <div class="controls">
      <Section label={$t("s.geom.round")}>
        {@render sliderRow(field("intensity"))}
        <Collapsible class="expander">
          <CollapsibleTrigger class="exp-trigger">
            <ChevronRight size={15} strokeWidth={2} />
            {$t("s.geom.allRadii")}
          </CollapsibleTrigger>
          <CollapsibleContent>
            {#each roundnessFull as f (f.key)}
              {@render sliderRow(f)}
            {/each}
          </CollapsibleContent>
        </Collapsible>
      </Section>

      <Section label={$t("s.geom.window")}>
        {@render sliderRow(field("window_corner"))}
        {@render sliderRow(field("border_width"))}
      </Section>

      <Section label={$t("s.geom.spacing")}>
        {@render sliderRow(field("density"))}
        <Collapsible class="expander">
          <CollapsibleTrigger class="exp-trigger">
            <ChevronRight size={15} strokeWidth={2} />
            {$t("s.geom.allSteps")}
          </CollapsibleTrigger>
          <CollapsibleContent>
            {#each spacingFull as f (f.key)}
              {@render sliderRow(f)}
            {/each}
          </CollapsibleContent>
        </Collapsible>
      </Section>

      <Section label={$t("s.geom.gaps")}>
        {@render sliderRow(field("gap"))}
        <OverrideRow
          label={$t("s.geom.smart")}
          hint={$t("s.geom.smartHint")}
          overridden={$smartGapsOverridden}
          onreset={resetSmartGaps}
          id="geom-smart-gaps"
        >
          {#snippet control()}
            <Switch value={$smartGaps} ariaLabel="Smart gaps" onchange={toggleSmartGaps} />
          {/snippet}
        </OverrideRow>
      </Section>
    </div>

    <aside class="preview-col">
      <div class="preview-sticky">
        <span class="preview-label">{$t("s.geom.preview")}</span>
        <div style={previewRadiusVars($effective)}>
          <ThemePreview colors={$colorsEffective} />
        </div>
        <div class="geom-samples">
          <div
            class="gs-window"
            style={`border-radius:${$effective.window_corner}px; border-width:${Math.max($effective.border_width, 1)}px; opacity:${$effective.border_width === 0 ? 0.45 : 1}`}
          >
            {$t("s.geom.winCorners")}
          </div>
          <div class="gs-tiling" style={`gap:${Math.max($effective.gap, 1)}px`}>
            <span></span>
            <span></span>
          </div>
        </div>
      </div>
    </aside>
    </div>
  </SectionGrid>
</Page>

<!-- One slider field with the shared override language. -->
{#snippet sliderRow(f: GeomField)}
  <OverrideRow
    label={$t(f.label)}
    hint={f.hint ? $t(f.hint) : ""}
    overridden={isOverridden($overrides, f.key)}
    onreset={() => resetGeom(f.key)}
    id={`geom-${f.key}`}
  >
    {#snippet control()}
      <ValueSlider
        value={displayVal(f, $effective)}
        min={f.min}
        max={f.max}
        step={f.step}
        unit={f.unit}
        ariaLabel={$t(f.label)}
        onchange={(v) => onSlide(f, v)}
      />
    {/snippet}
  </OverrideRow>
{/snippet}

<style>
  .editor {
    display: flex;
    flex-direction: column;
    gap: 1.5rem;
  }
  .controls {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    min-width: 0;
  }
  .preview-sticky {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .preview-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    padding-inline-start: 0.125rem;
  }
  .preview-col {
    order: -1;
  }

  /* Shape samples for window corners + tiling gaps, which the app strip can't
     show on its own. */
  .geom-samples {
    display: flex;
    gap: 0.5rem;
  }
  .gs-window {
    flex: 1;
    display: flex;
    align-items: flex-end;
    padding: 0.5rem;
    height: 3.5rem;
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
    border-style: solid;
    border-color: color-mix(in srgb, var(--color-accent, var(--foreground)) 55%, transparent);
  }
  .gs-tiling {
    flex: 1;
    display: flex;
    height: 3.5rem;
    padding: 0.375rem;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--foreground) 5%, transparent);
  }
  .gs-tiling span {
    flex: 1;
    border-radius: var(--radius-button, 6px);
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
  }

  /* The expander triggers (class rides the Collapsible root, so global). */
  :global(.exp-trigger) {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.5rem 0.25rem;
    border: none;
    background: transparent;
    font-size: var(--text-sm);
    font-weight: 500;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
    cursor: pointer;
  }
  :global(.exp-trigger:hover) {
    color: var(--foreground);
  }
  :global(.exp-trigger svg) {
    transition: transform var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  :global(.exp-trigger[data-state="open"] svg) {
    transform: rotate(90deg);
  }
</style>
