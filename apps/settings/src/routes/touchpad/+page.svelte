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
  import { t } from "$lib/i18n/messages";
  import { touchpad, load, set } from "$lib/stores/touchpad";

  const CLICK_METHODS = $derived([
    { value: "clickfinger", label: $t("s.touchpad.clickfinger") },
    { value: "areas", label: $t("s.touchpad.areas") },
  ]);

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
  title={$t("s.touchpad.title")}
  description={$t("s.touchpad.desc")}
>
  <SectionGrid>
    <Group label={$t("s.touchpad.clicking")}>
      <Row
        label={$t("s.touchpad.clickMethod")}
        description={$t("s.touchpad.clickMethod.desc")}
        id="touchpad-click-method"
      >
        {#snippet control()}
          <PopoverSelect
            value={$touchpad.config.click_method}
            options={CLICK_METHODS}
            onchange={(v) => set("click_method", v)}
            ariaLabel={$t("s.touchpad.clickMethod.aria")}
            width="280px"
          />
        {/snippet}
      </Row>
      <Row label={$t("s.touchpad.tapClick")} description={$t("s.touchpad.tapClick.desc")} id="touchpad-tap-to-click">
        {#snippet control()}
          <Switch value={$touchpad.config.tap_to_click} onchange={(v) => set("tap_to_click", v)} />
        {/snippet}
      </Row>
      <Row
        label={$t("s.touchpad.tapDrag")}
        description={$t("s.touchpad.tapDrag.desc")}
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
        label={$t("s.touchpad.disableTyping")}
        description={$t("s.touchpad.disableTyping.desc")}
        id="touchpad-disable-while-typing"
      >
        {#snippet control()}
          <Switch value={$touchpad.config.disable_while_typing} onchange={(v) => set("disable_while_typing", v)} />
        {/snippet}
      </Row>
    </Group>

    <Group label={$t("s.touchpad.scrolling")}>
      <Row label={$t("s.touchpad.twoFinger")} description={$t("s.touchpad.twoFinger.desc")} id="touchpad-two-finger-scroll">
        {#snippet control()}
          <Switch value={$touchpad.config.two_finger_scroll} onchange={(v) => set("two_finger_scroll", v)} />
        {/snippet}
      </Row>
      <Row label={$t("s.touchpad.naturalScroll")} description={$t("s.touchpad.naturalScroll.desc")} id="touchpad-natural-scroll">
        {#snippet control()}
          <Switch value={$touchpad.config.natural_scroll} onchange={(v) => set("natural_scroll", v)} />
        {/snippet}
      </Row>
    </Group>

    <Group label={$t("s.touchpad.pointer")}>
      <Row
        label={$t("s.mouse.accel")}
        description={$t("s.mouse.accel.desc")}
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
                ariaLabel={$t("s.mouse.accel")}
                oninput={(v) => set("acceleration", tickToAccel(v))}
              />
            </div>
            <span class="slider-value">{$touchpad.config.acceleration.toFixed(2)}</span>
          </div>
        {/snippet}
      </Row>
    </Group>

    {#if $touchpad.error}
      <div class="span-full error-box" title={$touchpad.error}>{$t("s.err.readPaused")}</div>
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
