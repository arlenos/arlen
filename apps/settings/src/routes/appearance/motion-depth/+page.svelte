<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Motion & Depth: the theme's transition speed + easing (with the real
  /// reduce-motion switch), and the shadow elevation + blur. The quiet tier, its
  /// own page. Same two-column split, override-row and live preview; a moving
  /// sample and a floating one, since neither shows in a static app strip. Rich
  /// by structure, not omission (appearance-surface.md).
  ///
  /// Mock-vs-live: reduce motion is live - it writes `appearance.toml
  /// [accessibility] reduce_motion`, which the shell reads. The durations,
  /// easing, shadows and blur are local-only and the page says so on screen.
  import { onMount } from "svelte";
  import { ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Section } from "@arlen/ui-kit/components/ui/section";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import ThemePreview from "$lib/components/appearance/ThemePreview.svelte";
  import { effective as colorsEffective } from "$lib/stores/themeColors";
  import {
    overrides,
    effective,
    isOverridden,
    setMd,
    resetMd,
    easingPresets,
    shadowPresets,
    easingBezier,
    shadowCss,
    loadMd,
  } from "$lib/stores/themeMotionDepth";
  import { theme } from "$lib/stores/theme";
  import { Notice } from "@arlen/ui-kit/components/ui/notice";

  const reduce = $derived(Boolean($effective.reduceMotion));

  onMount(() => {
    void theme.load().then(loadMd);
  });
  const durNormal = $derived(Number($effective.durationNormal));
  const easing = $derived(String($effective.easing));
  const shadow = $derived(String($effective.shadow));
  const blur = $derived(Boolean($effective.blurEnabled));

  // The motion sample transitions between two positions on a loop, using the
  // effective duration + easing, so the timing + curve read live.
  let pos = $state(false);
  onMount(() => {
    const id = setInterval(() => (pos = !pos), 1400);
    return () => clearInterval(id);
  });
</script>

<Page back={{ href: "/appearance", label: $t("s.nav.appearance") }}
  title={$t("s.md.title")}
  description={$t("s.md.desc")}
>
  <SectionGrid>
    <!-- The page draws the house values and its sliders write nothing. Said here
         rather than only in a source comment: a person reading numbers on a
         settings page takes them for their own. -->
    <Notice tone="neutral" class="span-full" text={$t("s.md.notApplied")} />
    <div class="editor span-full">
    <div class="controls">
      <Section label={$t("s.md.motion")}>
        <Row
          label={$t("s.md.reduce")}
          description={$t("s.md.reduceHint")}
          overridden={isOverridden($overrides, "reduceMotion")}
          onreset={() => resetMd("reduceMotion")}
          id="md-reduceMotion"
        >
          {#snippet control()}
            <Switch value={reduce} ariaLabel={$t("s.md.reduceMotion")} onchange={(v) => setMd("reduceMotion", v)} />
          {/snippet}
        </Row>
        <Row
          label={$t("s.md.speed")}
          description={$t("s.md.speedHint")}
          overridden={isOverridden($overrides, "durationNormal")}
          onreset={() => resetMd("durationNormal")}
          id="md-durationNormal"
        >
          {#snippet control()}
            <ValueSlider
              value={durNormal}
              min={60}
              max={400}
              step={20}
              unit="ms"
              ariaLabel={$t("s.md.speed")}
              onchange={(v) => setMd("durationNormal", v)}
            />
          {/snippet}
        </Row>
        <Collapsible class="expander">
          <CollapsibleTrigger class="exp-trigger">
            <ChevronRight size={15} strokeWidth={2} />
            {$t("s.md.allDurations")}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <Row
              label={$t("s.md.fast")}
              description={$t("s.md.fastHint")}
              overridden={isOverridden($overrides, "durationFast")}
              onreset={() => resetMd("durationFast")}
              id="md-durationFast"
            >
              {#snippet control()}
                <ValueSlider value={Number($effective.durationFast)} min={40} max={300} step={20} unit="ms" ariaLabel={$t("s.md.fast")} onchange={(v) => setMd("durationFast", v)} />
              {/snippet}
            </Row>
            <Row
              label={$t("s.md.slow")}
              description={$t("s.md.slowHint")}
              overridden={isOverridden($overrides, "durationSlow")}
              onreset={() => resetMd("durationSlow")}
              id="md-durationSlow"
            >
              {#snippet control()}
                <ValueSlider value={Number($effective.durationSlow)} min={200} max={800} step={20} unit="ms" ariaLabel={$t("s.md.slow")} onchange={(v) => setMd("durationSlow", v)} />
              {/snippet}
            </Row>
          </CollapsibleContent>
        </Collapsible>
        <Row
          label={$t("s.md.easing")}
          description={$t("s.md.easingHint")}
          overridden={isOverridden($overrides, "easing")}
          onreset={() => resetMd("easing")}
          id="md-easing"
        >
          {#snippet control()}
            <SegmentedControl value={easing} options={$easingPresets} ariaLabel={$t("s.md.easingAria")} onchange={(v) => setMd("easing", v)} />
          {/snippet}
        </Row>
      </Section>

      <Section label={$t("s.md.depth")}>
        <Row
          label={$t("s.md.shadows")}
          description={$t("s.md.shadowsHint")}
          overridden={isOverridden($overrides, "shadow")}
          onreset={() => resetMd("shadow")}
          id="md-shadow"
        >
          {#snippet control()}
            <SegmentedControl value={shadow} options={$shadowPresets} ariaLabel={$t("s.md.shadows")} onchange={(v) => setMd("shadow", v)} />
          {/snippet}
        </Row>
        <Row
          label={$t("s.md.blur")}
          description={$t("s.md.blurHint")}
          overridden={isOverridden($overrides, "blurEnabled")}
          onreset={() => resetMd("blurEnabled")}
          id="md-blurEnabled"
        >
          {#snippet control()}
            <Switch value={blur} ariaLabel={$t("s.md.blur")} onchange={(v) => setMd("blurEnabled", v)} />
          {/snippet}
        </Row>
      </Section>
    </div>

    <aside class="preview-col">
      <div class="preview-sticky">
        <span class="preview-label">{$t("s.md.preview")}</span>
        <ThemePreview colors={$colorsEffective} />

        <div class="md-sample">
          <span class="ms-caption">{reduce ? $t("s.md.capMotionReduced") : $t("s.md.capMotion")}</span>
          <span class="ms-track">
            <span
              class="ms-dot"
              style={`transition:${reduce ? "none" : `left ${durNormal}ms ${easingBezier(easing)}`}; left: ${pos && !reduce ? "calc(100% - 1.25rem)" : "0"}`}
            ></span>
          </span>
        </div>

        <div class="md-sample md-depth">
          <span class="ms-caption">{$t("s.md.capDepth")}</span>
          <span class="ds-stage">
            <span
              class="ds-card"
              style={`box-shadow:${shadowCss(shadow)}; ${blur ? "backdrop-filter: blur(6px); -webkit-backdrop-filter: blur(6px);" : ""}`}
            >
              {$t("s.md.card")}
            </span>
          </span>
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
    color: var(--color-fg-secondary, #a1a1aa);
    padding-inline-start: 0.125rem;
  }
  .preview-col {
    order: -1;
  }

  .md-sample {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .ms-caption {
    font-size: var(--text-2xs);
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .ms-track {
    position: relative;
    display: block;
    height: 1.25rem;
    padding: 0;
    border-radius: var(--radius-full);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .ms-dot {
    position: absolute;
    top: 0;
    left: 0;
    display: block;
    width: 1.25rem;
    height: 1.25rem;
    border-radius: var(--radius-full);
    background: var(--color-accent, var(--foreground));
  }

  /* The depth sample: a floating card over a soft ground so the shadow + frost
     read. */
  .md-depth .ds-stage {
    display: flex;
    justify-content: center;
    padding: 1.25rem 0.5rem;
    border-radius: var(--radius-input);
    background: linear-gradient(
      120deg,
      color-mix(in srgb, var(--color-accent, var(--foreground)) 22%, transparent),
      color-mix(in srgb, var(--foreground) 10%, transparent)
    );
  }
  .ds-card {
    padding: 0.625rem 1.5rem;
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    color: var(--foreground);
    font-size: var(--text-xs);
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
