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
    <!-- The page waits for the theme writes: only Reduce motion writes, so only
         it is offered (design-system.md §6.10). The preview stays. -->
    <Notice tone="neutral" class="span-full" text={$t("s.md.notApplied")} />
      <Section label={$t("s.md.motion")} class="span-full">
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
      </Section>
    <div class="preview span-full">
      <span class="preview-label">{$t("s.md.preview")}</span>
      <ThemePreview colors={$colorsEffective} />
    </div>
  </SectionGrid>
</Page>

<style>
  .preview-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: var(--color-fg-secondary, #a1a1aa);
    padding-inline-start: 0.125rem;
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
