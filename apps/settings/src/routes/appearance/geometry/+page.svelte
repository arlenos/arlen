<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Geometry: the theme's shape. A master Roundness knob up front; the per-radius
  /// bases, window corners, spacing and tiling gaps follow, with the granular
  /// per-token overrides behind expanders. Same two-column split, override-row and
  /// live preview as the Colours page (the preview corners round live as you drag).
  /// Rich by structure, not omission (appearance-surface.md).
  ///
  /// Mock-vs-live: STILL A FIXTURE. Every slider here moves a local store and
  /// writes nothing, and the numbers are the house defaults rather than this
  /// machine's. The backend for eleven of the sixteen fields exists now
  /// (`theme_resolved_metrics` / `theme_set_metric`); the reasons the other five
  /// are a separate job, and why wiring only eleven would be worse than this, are
  /// in `$lib/stores/themeGeometry.ts`.
  import { ChevronRight } from "lucide-svelte";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";
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
  import { Row } from "@arlen/ui-kit/components/ui/row";
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

<Page back={{ href: "/appearance", label: $t("s.nav.appearance") }}
  title={$t("s.geom.title")}
  description={$t("s.geom.desc")}
>
  <SectionGrid>
    <!-- The page waits: nothing here writes, so nothing here is offered
         (design-system.md §6.10). The preview stays, since it is the one thing
         that is true. -->
    <Notice tone="neutral" class="span-full" text={$t("s.geom.notApplied")} />
    <div class="preview span-full">
      <span class="preview-label">{$t("s.geom.preview")}</span>
      <div style={previewRadiusVars($effective)}>
        <ThemePreview colors={$colorsEffective} />
      </div>
    </div>
  </SectionGrid>
</Page>

<!-- One slider field with the shared override language. -->
{#snippet sliderRow(f: GeomField)}
  <Row
    label={$t(f.label)}
    description={f.hint ? $t(f.hint) : ""}
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
  </Row>
{/snippet}

<style>
  .preview-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--color-fg-secondary, #a1a1aa);
    padding-inline-start: 0.125rem;
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
