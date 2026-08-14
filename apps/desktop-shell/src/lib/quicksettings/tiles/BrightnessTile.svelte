<script lang="ts">
  import { t } from "$lib/i18n/messages";
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
  import { shellAction } from "$lib/shellAction";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";

  let percent = $state(100);
  let device = $state<string | null>(null);
  let supported = $state(false);
  let writeTimer: ReturnType<typeof setTimeout> | null = null;

  /// Reads the position off the hardware. The backlight owns the value, so this
  /// is what settles the slider - both on mount and after a write, because
  /// `arlen://brightness-changed` is emitted by the Fn-key path only and says
  /// nothing about whether the slider's own write landed.
  async function readHardware(): Promise<void> {
    try {
      const dev = await invoke<{
        name: string;
        max: number;
        current: number;
        kind: string;
      } | null>("brightness_get_primary");
      if (!dev) {
        supported = false;
        return;
      }
      supported = true;
      device = dev.name;
      const linear = dev.max > 0 ? dev.current / dev.max : 0;
      // Inverse gamma curve so the slider position matches perception.
      percent = Math.round(Math.pow(linear, 1 / 2.2) * 100);
    } catch {
      supported = false;
    }
  }

  onMount(() => {
    void readHardware();

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
      // Settle on the hardware afterwards. Dragging moves the handle first
      // because anything else feels broken, but that position is a guess until
      // the backlight confirms it - and a refused write used to leave the
      // slider sitting at a brightness the screen never had.
      void shellAction(
        "brightness_set",
        { device: dev, value: fraction },
        "sh.tile.errBrightness",
      ).then(() => void readHardware());
    }, 32);
  }
</script>

{#if supported}
  <SliderTile
    label={$t("sh.tile.brightness")}
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
