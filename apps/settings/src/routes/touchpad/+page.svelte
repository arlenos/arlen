<script lang="ts">
  import { onMount } from "svelte";
  import { Slider } from "$lib/components/ui/slider";
  import { PopoverSelect } from "$lib/components/ui/popover-select";
  import Switch from "$lib/components/ui/switch/switch.svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsGroup from "$lib/components/settings/SettingsGroup.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
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

<SettingsPage
  title="Touchpad"
  description="Gesture, scroll, and acceleration defaults for integrated trackpads."
>
  <SettingsGroup label="Clicking">
    <SettingsRow
      label="Click method"
      description="How multi-finger clicks are turned into mouse buttons."
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
    </SettingsRow>

    <SettingsRow
      label="Tap to click"
      description="Single-finger tap acts as a primary click."
    >
      {#snippet control()}
        <Switch
          value={$touchpad.config.tap_to_click}
          onchange={(v) => set("tap_to_click", v)}
        />
      {/snippet}
    </SettingsRow>

    <SettingsRow
      label="Tap and drag"
      description="Tap, hold, and drag to move windows or select text. Requires Tap to click."
    >
      {#snippet control()}
        <Switch
          value={$touchpad.config.tap_drag}
          onchange={(v) => set("tap_drag", v)}
          disabled={!$touchpad.config.tap_to_click}
        />
      {/snippet}
    </SettingsRow>

    <SettingsRow
      label="Disable while typing"
      description="Ignore touchpad input briefly after each keystroke."
    >
      {#snippet control()}
        <Switch
          value={$touchpad.config.disable_while_typing}
          onchange={(v) => set("disable_while_typing", v)}
        />
      {/snippet}
    </SettingsRow>
  </SettingsGroup>

  <SettingsGroup label="Scrolling">
    <SettingsRow
      label="Two-finger scroll"
      description="Scroll by dragging two fingers on the touchpad."
    >
      {#snippet control()}
        <Switch
          value={$touchpad.config.two_finger_scroll}
          onchange={(v) => set("two_finger_scroll", v)}
        />
      {/snippet}
    </SettingsRow>

    <SettingsRow
      label="Natural scroll"
      description="Content follows finger direction (macOS-style)."
    >
      {#snippet control()}
        <Switch
          value={$touchpad.config.natural_scroll}
          onchange={(v) => set("natural_scroll", v)}
        />
      {/snippet}
    </SettingsRow>
  </SettingsGroup>

  <SettingsGroup label="Pointer">
    <SettingsRow
      label="Acceleration"
      description="Negative values slow the pointer; positive speed it up."
    >
      {#snippet control()}
        <div class="flex items-center gap-3">
          <Slider
            min={0}
            max={100}
            step={1}
            value={accelToTick($touchpad.config.acceleration)}
            onValueChange={(v) => set("acceleration", tickToAccel(v))}
            class="w-40"
          />
          <span
            class="min-w-12 text-right font-mono text-xs text-muted-foreground"
          >
            {$touchpad.config.acceleration.toFixed(2)}
          </span>
        </div>
      {/snippet}
    </SettingsRow>
  </SettingsGroup>

  {#if $touchpad.error}
    <div
      class="rounded-[var(--radius-chip)] border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive"
    >
      {$touchpad.error}
    </div>
  {/if}
</SettingsPage>
