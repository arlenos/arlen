<script lang="ts">
  /// Motion & Depth: the theme's transition speed + easing (with the real
  /// reduce-motion switch), and the shadow elevation + blur. The quiet tier, its
  /// own page. Same two-column split, override-row and live preview; a moving
  /// sample and a floating one, since neither shows in a static app strip. Rich
  /// by structure, not omission (appearance-surface.md).
  ///
  /// Mock-vs-live: `reduce_motion` is real (`set_reduce_motion`); durations /
  /// easing / shadows / blur need the theme.toml override backend. Fixture until.
  import { onMount } from "svelte";
  import { ChevronRight } from "lucide-svelte";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { ValueSlider } from "@arlen/ui-kit/components/ui/value-slider";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { SegmentedControl } from "@arlen/ui-kit/components/ui/segmented-control";
  import {
    Collapsible,
    CollapsibleTrigger,
    CollapsibleContent,
  } from "@arlen/ui-kit/components/ui/collapsible";
  import OverrideRow from "$lib/components/appearance/OverrideRow.svelte";
  import ThemePreview from "$lib/components/appearance/ThemePreview.svelte";
  import { effective as colorsEffective } from "$lib/stores/themeColors";
  import {
    overrides,
    effective,
    isOverridden,
    setMd,
    resetMd,
    EASING_PRESETS,
    SHADOW_PRESETS,
    easingBezier,
    shadowCss,
  } from "$lib/stores/themeMotionDepth";

  const reduce = $derived(Boolean($effective.reduceMotion));
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

<Page
  title="Motion & Depth"
  description="How things move and lift: transition speed, easing, shadows, and blur. Change one and it overrides just that value, on top of the theme."
>
  <SectionGrid>
    <div class="editor span-full">
    <div class="controls">
      <Group label="Motion">
        <OverrideRow
          label="Reduce motion"
          hint="Turn off animation across the desktop"
          overridden={isOverridden($overrides, "reduceMotion")}
          onreset={() => resetMd("reduceMotion")}
          id="md-reduceMotion"
        >
          {#snippet control()}
            <Switch value={reduce} ariaLabel="Reduce motion" onchange={(v) => setMd("reduceMotion", v)} />
          {/snippet}
        </OverrideRow>
        <OverrideRow
          label="Speed"
          hint="The base transition duration"
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
              ariaLabel="Speed"
              onchange={(v) => setMd("durationNormal", v)}
            />
          {/snippet}
        </OverrideRow>
        <Collapsible class="expander">
          <CollapsibleTrigger class="exp-trigger">
            <ChevronRight size={15} strokeWidth={2} />
            All durations
          </CollapsibleTrigger>
          <CollapsibleContent>
            <OverrideRow
              label="Fast"
              hint="Quick feedback, like a hover"
              overridden={isOverridden($overrides, "durationFast")}
              onreset={() => resetMd("durationFast")}
              id="md-durationFast"
            >
              {#snippet control()}
                <ValueSlider value={Number($effective.durationFast)} min={40} max={300} step={20} unit="ms" ariaLabel="Fast" onchange={(v) => setMd("durationFast", v)} />
              {/snippet}
            </OverrideRow>
            <OverrideRow
              label="Slow"
              hint="Larger movements, like a panel"
              overridden={isOverridden($overrides, "durationSlow")}
              onreset={() => resetMd("durationSlow")}
              id="md-durationSlow"
            >
              {#snippet control()}
                <ValueSlider value={Number($effective.durationSlow)} min={200} max={800} step={20} unit="ms" ariaLabel="Slow" onchange={(v) => setMd("durationSlow", v)} />
              {/snippet}
            </OverrideRow>
          </CollapsibleContent>
        </Collapsible>
        <OverrideRow
          label="Easing"
          hint="The curve a movement follows"
          overridden={isOverridden($overrides, "easing")}
          onreset={() => resetMd("easing")}
          id="md-easing"
        >
          {#snippet control()}
            <SegmentedControl value={easing} options={EASING_PRESETS} ariaLabel="Easing" onchange={(v) => setMd("easing", v)} />
          {/snippet}
        </OverrideRow>
      </Group>

      <Group label="Depth">
        <OverrideRow
          label="Shadows"
          hint="How much things lift off the surface"
          overridden={isOverridden($overrides, "shadow")}
          onreset={() => resetMd("shadow")}
          id="md-shadow"
        >
          {#snippet control()}
            <SegmentedControl value={shadow} options={SHADOW_PRESETS} ariaLabel="Shadows" onchange={(v) => setMd("shadow", v)} />
          {/snippet}
        </OverrideRow>
        <OverrideRow
          label="Blur"
          hint="Frost behind menus and overlays"
          overridden={isOverridden($overrides, "blurEnabled")}
          onreset={() => resetMd("blurEnabled")}
          id="md-blurEnabled"
        >
          {#snippet control()}
            <Switch value={blur} ariaLabel="Blur" onchange={(v) => setMd("blurEnabled", v)} />
          {/snippet}
        </OverrideRow>
      </Group>
    </div>

    <aside class="preview-col">
      <div class="preview-sticky">
        <span class="preview-label">Live preview</span>
        <ThemePreview colors={$colorsEffective} />

        <div class="md-sample">
          <span class="ms-caption">Motion{reduce ? " (reduced)" : ""}</span>
          <span class="ms-track">
            <span
              class="ms-dot"
              style={`transition:${reduce ? "none" : `left ${durNormal}ms ${easingBezier(easing)}`}; left: ${pos && !reduce ? "calc(100% - 1.25rem)" : "0"}`}
            ></span>
          </span>
        </div>

        <div class="md-sample md-depth">
          <span class="ms-caption">Depth</span>
          <span class="ds-stage">
            <span
              class="ds-card"
              style={`box-shadow:${shadowCss(shadow)}; ${blur ? "backdrop-filter: blur(6px); -webkit-backdrop-filter: blur(6px);" : ""}`}
            >
              Card
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
    display: grid;
    grid-template-columns: minmax(0, 1fr) 20rem;
    gap: 1.5rem;
  }
  .controls {
    display: flex;
    flex-direction: column;
    gap: 1rem;
    min-width: 0;
  }
  .preview-sticky {
    position: sticky;
    top: 0;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }
  .preview-label {
    font-size: 0.6875rem;
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 45%, transparent);
    padding-left: 0.125rem;
  }
  @media (max-width: 60rem) {
    .editor {
      grid-template-columns: 1fr;
    }
    .preview-col {
      order: -1;
    }
    .preview-sticky {
      position: static;
    }
  }

  .md-sample {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    padding: 0.75rem 1rem;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--foreground) 4%, transparent);
    border: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .ms-caption {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--foreground) 50%, transparent);
  }
  .ms-track {
    position: relative;
    display: block;
    height: 1.25rem;
    padding: 0;
    border-radius: var(--radius-full, 9999px);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }
  .ms-dot {
    position: absolute;
    top: 0;
    left: 0;
    display: block;
    width: 1.25rem;
    height: 1.25rem;
    border-radius: var(--radius-full, 9999px);
    background: var(--color-accent, var(--foreground));
  }

  /* The depth sample: a floating card over a soft ground so the shadow + frost
     read. */
  .md-depth .ds-stage {
    display: flex;
    justify-content: center;
    padding: 1.25rem 0.5rem;
    border-radius: var(--radius-input, 8px);
    background: linear-gradient(
      120deg,
      color-mix(in srgb, var(--color-accent, var(--foreground)) 22%, transparent),
      color-mix(in srgb, var(--foreground) 10%, transparent)
    );
  }
  .ds-card {
    padding: 0.625rem 1.5rem;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    color: var(--foreground);
    font-size: 0.75rem;
  }

  /* The expander trigger (class rides the Collapsible root, so global). */
  :global(.exp-trigger) {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.5rem 0.25rem;
    border: none;
    background: transparent;
    font-size: 0.8125rem;
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
