<script lang="ts">
  /// Touchpad settings page — on the design-system canon (Page/SectionGrid/Group/
  /// Row/Switch from `@arlen/ui-kit`; FillSlider/PopoverSelect app-local).

  import { onMount } from "svelte";
  import { FillSlider } from "@arlen/ui-kit/components/ui/fill-slider";
  import { PopoverSelect } from "@arlen/ui-kit/components/ui/popover-select";
  import { Page } from "@arlen/ui-kit/components/ui/page";
  import { SectionGrid } from "@arlen/ui-kit/components/ui/section-grid";
  import { Group } from "@arlen/ui-kit/components/ui/group";
  import { Row } from "@arlen/ui-kit/components/ui/row";
  import { Switch } from "@arlen/ui-kit/components/ui/switch";
  import { touchpad, load, set } from "$lib/stores/touchpad";

  const CLICK_METHODS = [
    {
      value: "clickfinger",
      label: "Click with fingers (1 = left, 2 = right, 3 = middle)",
    },
    { value: "areas", label: "Click areas (corners of the touchpad)" },
  ];

  onMount(() => {
    void load();
  });

  function accelToTick(v: number): number {
    return Math.round(((v + 1) / 2) * 100);
  }
  function tickToAccel(t: number): number {
    return Math.max(-1, Math.min(1, (t / 100) * 2 - 1));
  }
</script>

<Page
  title="Touchpad"
  description="Gesture, scroll, and acceleration defaults for integrated trackpads."
>
  <SectionGrid>
    <Group label="Clicking">
      <Row
        label="Click method"
        description="How multi-finger clicks are turned into mouse buttons."
        id="touchpad-click-method"
      >
        {#snippet control()}
          <PopoverSelect
            value={$touchpad.config.click_method}
            options={CLICK_METHODS}
            onchange={(v) => set("click_method", v)}
            ariaLabel="Touchpad click method"
            width="280px"
          />
        {/snippet}
      </Row>
      <Row label="Tap to click" description="Single-finger tap acts as a primary click." id="touchpad-tap-to-click">
        {#snippet control()}
          <Switch value={$touchpad.config.tap_to_click} onchange={(v) => set("tap_to_click", v)} />
        {/snippet}
      </Row>
      <Row
        label="Tap and drag"
        description="Tap, hold, and drag to move windows or select text. Requires Tap to click."
        id="touchpad-tap-drag"
      >
        {#snippet control()}
          <Switch
            value={$touchpad.config.tap_drag}
            onchange={(v) => set("tap_drag", v)}
            disabled={!$touchpad.config.tap_to_click}
          />
        {/snippet}
      </Row>
      <Row
        label="Disable while typing"
        description="Ignore touchpad input briefly after each keystroke."
        id="touchpad-disable-while-typing"
      >
        {#snippet control()}
          <Switch value={$touchpad.config.disable_while_typing} onchange={(v) => set("disable_while_typing", v)} />
        {/snippet}
      </Row>
    </Group>

    <Group label="Scrolling">
      <Row label="Two-finger scroll" description="Scroll by dragging two fingers on the touchpad." id="touchpad-two-finger-scroll">
        {#snippet control()}
          <Switch value={$touchpad.config.two_finger_scroll} onchange={(v) => set("two_finger_scroll", v)} />
        {/snippet}
      </Row>
      <Row label="Natural scroll" description="Content follows finger direction (macOS-style)." id="touchpad-natural-scroll">
        {#snippet control()}
          <Switch value={$touchpad.config.natural_scroll} onchange={(v) => set("natural_scroll", v)} />
        {/snippet}
      </Row>
    </Group>

    <Group label="Pointer">
      <Row
        label="Acceleration"
        description="Negative values slow the pointer; positive speed it up."
        id="touchpad-acceleration"
      >
        {#snippet control()}
          <div class="slider-cell">
            <div class="slider-track">
              <FillSlider
                min={0}
                max={100}
                step={1}
                value={accelToTick($touchpad.config.acceleration)}
                ariaLabel="Acceleration"
                oninput={(v) => set("acceleration", tickToAccel(v))}
              />
            </div>
            <span class="slider-value">{$touchpad.config.acceleration.toFixed(2)}</span>
          </div>
        {/snippet}
      </Row>
    </Group>

    {#if $touchpad.error}
      <div class="span-full error-box">{$touchpad.error}</div>
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
