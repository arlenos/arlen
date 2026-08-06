<script lang="ts">
  import { t } from "$lib/i18n/messages";
  import { sysOptions } from "$lib/stores/themeSystem";
  /// Typography: the theme's fonts, size, line height, and weights. The everyday
  /// fonts + size up front; the weights behind an expander. Same two-column split,
  /// override-row and live preview as the other suite pages. A dedicated type
  /// sample carries the size / line height / weight, which the app strip can't
  /// show alone. Rich by structure, not omission (appearance-surface.md).
  ///
  /// Mock-vs-live: fonts + size are real config keys; line height + weights need
  /// the theme.toml override backend, and the font list is fixed (fc-list is a
  /// coder gap). Fixture-backed until those land.
  import { ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import OverrideRow from "$lib/components/appearance/OverrideRow.svelte";
  import ThemePreview from "$lib/components/appearance/ThemePreview.svelte";
  import FontSelect from "$lib/components/appearance/FontSelect.svelte";
  import { effective as colorsEffective } from "$lib/stores/themeColors";
  import { FONT_OPTIONS, MONO_FONT_OPTIONS } from "$lib/stores/theme";
  import {
    overrides,
    effective,
    isOverridden,
    setTypo,
    resetTypo,
  } from "$lib/stores/themeTypography";

  const sans = $derived(String($effective.fontSans));
  const mono = $derived(String($effective.fontMono));
  const size = $derived(Number($effective.sizeBase));
  const lh = $derived(Number($effective.lineHeight));
  const wNormal = $derived(Number($effective.weightNormal));
  const wMedium = $derived(Number($effective.weightMedium));
  const wBold = $derived(Number($effective.weightBold));

  const WEIGHTS = [
    { key: "weightNormal", labelKey: "s.typo.weightNormal", hintKey: "s.typo.weightNormalHint" },
    { key: "weightMedium", labelKey: "s.typo.weightMedium", hintKey: "s.typo.weightMediumHint" },
    { key: "weightBold", labelKey: "s.typo.weightBold", hintKey: "s.typo.weightBoldHint" },
  ];
</script>

<Page
  title={$t("s.typo.title")}
  description={$t("s.typo.desc")}
>
  <SectionGrid>
    <div class="editor span-full">
    <div class="controls">
      <Section label={$t("s.typo.fonts")}>
        <OverrideRow
          label={$t("s.typo.interface")}
          hint={$t("s.typo.interfaceHint")}
          overridden={isOverridden($overrides, "fontSans")}
          onreset={() => resetTypo("fontSans")}
          id="typo-fontSans"
        >
          {#snippet control()}
            <FontSelect
              value={sans}
              options={sysOptions(FONT_OPTIONS, $t)}
              ariaLabel={$t("s.typo.interface")}
              onchange={(v) => setTypo("fontSans", v)}
            />
          {/snippet}
        </OverrideRow>
        <OverrideRow
          label={$t("s.typo.mono")}
          hint={$t("s.typo.monoHint")}
          overridden={isOverridden($overrides, "fontMono")}
          onreset={() => resetTypo("fontMono")}
          id="typo-fontMono"
        >
          {#snippet control()}
            <FontSelect
              value={mono}
              options={sysOptions(MONO_FONT_OPTIONS, $t)}
              ariaLabel={$t("s.typo.mono")}
              onchange={(v) => setTypo("fontMono", v)}
            />
          {/snippet}
        </OverrideRow>
      </Section>

      <Section label={$t("s.typo.size")}>
        <OverrideRow
          label={$t("s.typo.baseSize")}
          hint={$t("s.typo.baseSizeHint")}
          overridden={isOverridden($overrides, "sizeBase")}
          onreset={() => resetTypo("sizeBase")}
          id="typo-sizeBase"
        >
          {#snippet control()}
            <ValueSlider
              value={size}
              min={12}
              max={18}
              step={1}
              unit="px"
              ariaLabel={$t("s.typo.baseSize")}
              onchange={(v) => setTypo("sizeBase", v)}
            />
          {/snippet}
        </OverrideRow>
        <OverrideRow
          label={$t("s.typo.lineHeight")}
          hint={$t("s.typo.lineHeightHint")}
          overridden={isOverridden($overrides, "lineHeight")}
          onreset={() => resetTypo("lineHeight")}
          id="typo-lineHeight"
        >
          {#snippet control()}
            <ValueSlider
              value={lh}
              min={1.1}
              max={1.9}
              step={0.05}
              ariaLabel={$t("s.typo.lineHeight")}
              onchange={(v) => setTypo("lineHeight", v)}
            />
          {/snippet}
        </OverrideRow>
      </Section>

      <Collapsible class="expander">
        <CollapsibleTrigger class="exp-trigger">
          <ChevronRight size={15} strokeWidth={2} />
          {$t("s.typo.weights")}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <Section>
            {#each WEIGHTS as w (w.key)}
              <OverrideRow
                label={$t(w.labelKey)}
                hint={$t(w.hintKey)}
                overridden={isOverridden($overrides, w.key)}
                onreset={() => resetTypo(w.key)}
                id={`typo-${w.key}`}
              >
                {#snippet control()}
                  <ValueSlider
                    value={Number($effective[w.key])}
                    min={300}
                    max={900}
                    step={100}
                    ariaLabel={$t("s.typo.weightAria", { name: $t(w.labelKey) })}
                    onchange={(v) => setTypo(w.key, v)}
                  />
                {/snippet}
              </OverrideRow>
            {/each}
          </Section>
        </CollapsibleContent>
      </Collapsible>
    </div>

    <aside class="preview-col">
      <div class="preview-sticky">
        <span class="preview-label">{$t("s.typo.preview")}</span>
        <div style={`font-family:'${sans}', ui-sans-serif, system-ui, sans-serif`}>
          <ThemePreview colors={$colorsEffective} />
        </div>
        <div
          class="type-sample"
          style={`font-family:'${sans}', ui-sans-serif, system-ui, sans-serif; font-size:${size}px; line-height:${lh}`}
        >
          <div class="ts-h" style={`font-weight:${wBold}`}>{$t("s.typo.sampleHead")}</div>
          <p class="ts-p" style={`font-weight:${wNormal}`}>
            {$t("s.typo.sampleBody")}
          </p>
          <div class="ts-med" style={`font-weight:${wMedium}`}>{$t("s.typo.sampleMedium")}</div>
          <!-- Left untranslated on purpose: this shows the monospace face, and
               code looks the same whatever the UI language. Listed in the i18n
               baseline so the gate agrees. -->
          <code class="ts-mono" style={`font-family:'${mono}', ui-monospace, monospace`}>
            const answer = 42;
          </code>
        </div>
      </div>
    </aside>
    </div>
  </SectionGrid>
</Page>

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

  /* The type sample carries size / line height / weight, which the app strip
     can't show on its own. */
  .type-sample {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.875rem 1rem;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  .ts-h {
    font-size: 1.35em;
  }
  .ts-p {
    margin: 0;
    color: color-mix(in srgb, var(--foreground) 78%, transparent);
  }
  .ts-med {
    font-size: 0.9em;
  }
  .ts-mono {
    font-size: 0.9em;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
  }

  /* The expander trigger (class rides the Collapsible root, so global). */
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
