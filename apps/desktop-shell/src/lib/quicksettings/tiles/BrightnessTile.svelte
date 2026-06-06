<script lang="ts">
  /// QS tile: Display brightness slider.
  ///
  /// Reads the live hardware fraction on mount, listens for the
  /// `arlen://brightness-changed` event so the slider tracks the
  /// hardware Fn-row keys, and coalesces drag updates into 30Hz
  /// hardware writes via a 32ms timer (matches the pattern in the
  /// old QS panel).
  import { SliderTile } from "@arlen/ui-kit/components/quicksettings";
  import { Sun } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let percent = $state(100);
  let device = $state<string | null>(null);
  let supported = $state(false);
  let writeTimer: ReturnType<typeof setTimeout> | null = null;

  onMount(() => {
    invoke<{ name: string; max: number; current: number; kind: string } | null>(
      "brightness_get_primary",
    )
      .then((dev) => {
        if (!dev) {
          supported = false;
          return;
        }
        supported = true;
        device = dev.name;
        const linear = dev.max > 0 ? dev.current / dev.max : 0;
        // Inverse gamma curve so the slider position matches perception.
        percent = Math.round(Math.pow(linear, 1 / 2.2) * 100);
      })
      .catch(() => (supported = false));

    let stop: UnlistenFn | null = null;
    listen<{ device: string; fraction: number }>(
      "arlen://brightness-changed",
      ({ payload }) => {
        percent = Math.round(payload.fraction * 100);
      },
    ).then((u) => (stop = u));

    return () => {
      if (writeTimer) clearTimeout(writeTimer);
      stop?.();
    };
  });

  function handleInput(value: number) {
    percent = value;
    if (!supported || !device) return;
    if (writeTimer) clearTimeout(writeTimer);
    const dev = device;
    const fraction = value / 100;
    writeTimer = setTimeout(() => {
      invoke("brightness_set", { device: dev, value: fraction }).catch(() => {});
    }, 32);
  }
</script>

{#if supported}
  <SliderTile
    label="Brightness"
    value={percent}
    min={0}
    max={100}
    oninput={handleInput}
  >
    {#snippet icon()}
      <Sun size={16} strokeWidth={1.75} />
    {/snippet}
  </SliderTile>
{/if}
