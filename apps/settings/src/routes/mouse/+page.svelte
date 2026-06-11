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
  title="Mouse"
  description="Pointer acceleration and scroll direction for external mice."
>
  <SectionGrid>
    <Group label="Behavior">
      <Row
        label="Acceleration"
        description="Negative values slow the pointer; positive speed it up."
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
                ariaLabel="Acceleration"
                oninput={(v) => set("acceleration", tickToAccel(v))}
              />
            </div>
            <span class="slider-value">{$mouse.config.acceleration.toFixed(2)}</span>
          </div>
        {/snippet}
      </Row>

      <Row
        label="Natural scroll"
        description="Scroll direction follows finger or wheel movement."
        id="mouse-natural-scroll"
      >
        {#snippet control()}
          <Switch value={$mouse.config.natural_scroll} onchange={(v) => set("natural_scroll", v)} />
        {/snippet}
      </Row>

      <Row label="Left-handed" description="Swap left and right mouse buttons." id="mouse-left-handed">
        {#snippet control()}
          <Switch value={$mouse.config.left_handed} onchange={(v) => set("left_handed", v)} />
        {/snippet}
      </Row>

      <Row
        label="Scroll speed"
        description="Multiplier on wheel scroll deltas. 1.0 is the libinput default."
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
                ariaLabel="Scroll speed"
                oninput={(v) => set("scroll_speed", v / 100)}
              />
            </div>
            <span class="slider-value">{$mouse.config.scroll_speed.toFixed(1)}×</span>
          </div>
        {/snippet}
      </Row>
    </Group>

    {#if $mouse.error}
      <div class="span-full error-box" title={$mouse.error}>Can't read these settings right now. Changes are paused.</div>
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
    text-align: right;
    font-variant-numeric: tabular-nums;
    font-size: 0.75rem;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .error-box {
    padding: 0.75rem;
    border-radius: var(--radius-chip, 4px);
    border: 1px solid color-mix(in srgb, var(--destructive) 40%, transparent);
    background: color-mix(in srgb, var(--destructive) 10%, transparent);
    font-size: 0.8125rem;
    color: var(--destructive);
  }
</style>
