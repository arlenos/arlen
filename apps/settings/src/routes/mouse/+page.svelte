<script lang="ts">
  import { onMount } from "svelte";
  import { Slider } from "$lib/components/ui/slider";
  import Switch from "$lib/components/ui/switch/switch.svelte";
  import SettingsPage from "$lib/components/settings/SettingsPage.svelte";
  import SettingsGroup from "$lib/components/settings/SettingsGroup.svelte";
  import SettingsRow from "$lib/components/settings/SettingsRow.svelte";
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

<SettingsPage
  title="Mouse"
  description="Pointer acceleration and scroll direction for external mice."
>
  <SettingsGroup label="Behavior">
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
            value={accelToTick($mouse.config.acceleration)}
            onValueChange={(v) => set("acceleration", tickToAccel(v))}
            class="w-40"
          />
          <span
            class="min-w-12 text-right font-mono text-xs text-muted-foreground"
          >
            {$mouse.config.acceleration.toFixed(2)}
          </span>
        </div>
      {/snippet}
    </SettingsRow>

    <SettingsRow
      label="Natural scroll"
      description="Scroll direction follows finger or wheel movement."
    >
      {#snippet control()}
        <Switch
          value={$mouse.config.natural_scroll}
          onchange={(v) => set("natural_scroll", v)}
        />
      {/snippet}
    </SettingsRow>

    <SettingsRow
      label="Left-handed"
      description="Swap left and right mouse buttons."
    >
      {#snippet control()}
        <Switch
          value={$mouse.config.left_handed}
          onchange={(v) => set("left_handed", v)}
        />
      {/snippet}
    </SettingsRow>

    <SettingsRow
      label="Scroll speed"
      description="Multiplier on wheel scroll deltas. 1.0 is the libinput default."
    >
      {#snippet control()}
        <div class="flex items-center gap-3">
          <Slider
            min={10}
            max={300}
            step={10}
            value={Math.round($mouse.config.scroll_speed * 100)}
            onValueChange={(v) => set("scroll_speed", v / 100)}
            class="w-40"
          />
          <span
            class="min-w-12 text-right font-mono text-xs text-muted-foreground"
          >
            {$mouse.config.scroll_speed.toFixed(1)}×
          </span>
        </div>
      {/snippet}
    </SettingsRow>
  </SettingsGroup>

  {#if $mouse.error}
    <div
      class="rounded-[var(--radius-chip)] border border-destructive/40 bg-destructive/10 p-3 text-sm text-destructive"
    >
      {$mouse.error}
    </div>
  {/if}
</SettingsPage>
