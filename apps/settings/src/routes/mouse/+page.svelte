<script lang="ts">
  /// Mouse settings page — on the design-system canon (Page/SectionGrid/Group/
  /// Row/Switch from `@arlen/ui-kit`; FillSlider app-local for now).

  import { onMount } from "svelte";
  import { FillSlider } from "@arlen/ui-kit/components/ui/fill-slider";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { t } from "$lib/i18n/messages";
  import { mouse, load, set } from "$lib/stores/mouse";

  onMount(() => {
    void load();
  });

  /// Slider works in 0..1 space so we map acceleration (-1..1) to ticks.
  function accelToTick(v: number): number {
    return Math.round(((v + 1) / 2) * 100);
  }
  function tickToAccel(t: number): number {
    return Math.max(-1, Math.min(1, (t / 100) * 2 - 1));
  }
</script>

<Page
  title={$t("s.mouse.title")}
  description={$t("s.mouse.desc")}
>
  <SectionGrid>
    <Group label={$t("s.mouse.behavior")} class="span-full">
      <Row
        label={$t("s.mouse.accel")}
        description={$t("s.mouse.accel.desc")}
        id="mouse-acceleration"
      >
        {#snippet control()}
          <div class="slider-cell">
            <div class="slider-track">
              <FillSlider
                min={0}
                max={100}
                step={1}
                value={accelToTick($mouse.config.acceleration)}
                ariaLabel={$t("s.mouse.accel")}
                oninput={(v) => set("acceleration", tickToAccel(v))}
              />
            </div>
            <span class="slider-value">{$mouse.config.acceleration.toFixed(2)}</span>
          </div>
        {/snippet}
      </Row>

      <Row
        label={$t("s.mouse.naturalScroll")}
        description={$t("s.mouse.naturalScroll.desc")}
        id="mouse-natural-scroll"
      >
        {#snippet control()}
          <Switch value={$mouse.config.natural_scroll} onchange={(v) => set("natural_scroll", v)} />
        {/snippet}
      </Row>

      <Row label={$t("s.mouse.leftHanded")} description={$t("s.mouse.leftHanded.desc")} id="mouse-left-handed">
        {#snippet control()}
          <Switch value={$mouse.config.left_handed} onchange={(v) => set("left_handed", v)} />
        {/snippet}
      </Row>

      <Row
        label={$t("s.mouse.scrollSpeed")}
        description={$t("s.mouse.scrollSpeed.desc")}
        id="mouse-scroll-speed"
      >
        {#snippet control()}
          <div class="slider-cell">
            <div class="slider-track">
              <FillSlider
                min={10}
                max={300}
                step={10}
                value={Math.round($mouse.config.scroll_speed * 100)}
                ariaLabel={$t("s.mouse.scrollSpeed")}
                oninput={(v) => set("scroll_speed", v / 100)}
              />
            </div>
            <span class="slider-value">{$mouse.config.scroll_speed.toFixed(1)}×</span>
          </div>
        {/snippet}
      </Row>
    </Group>

    {#if $mouse.error}
      <div class="span-full error-box" title={$mouse.error}>{$t("s.err.readPaused")}</div>
    {/if}
  </SectionGrid>
</Page>

<style>
  .slider-cell {
    display: flex;
    align-items: center;
    gap: 0.75rem;
  }
  .slider-track {
    width: 10rem;
  }
  .slider-value {
    min-width: 3rem;
    text-align: end;
    font-variant-numeric: tabular-nums;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .error-box {
    padding: 0.75rem;
    border-radius: var(--radius-chip, 4px);
    border: 1px solid color-mix(in srgb, var(--destructive) 40%, transparent);
    background: color-mix(in srgb, var(--destructive) 10%, transparent);
    font-size: var(--text-sm);
    color: var(--destructive);
  }
</style>
