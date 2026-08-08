<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// QS tile: Output volume slider + secondary tap to AudioPopover.
  ///
  /// Slider is inline. The leading icon button opens the existing
  /// AudioPopover for output picker / per-app mixer / mute / input.
  import { SliderTile } from "@arlen/ui-kit/components/quicksettings";
  import { Volume2, VolumeX, Volume1, Headphones, Speaker } from "lucide-svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { openPopover } from "$lib/stores/activePopover.js";

  interface AudioStatus {
    volume: number;
    muted: boolean;
    output_type: string;
  }

  // The initial value is a placeholder, not a reading: `volume: 0` renders as
  // "0%" next to a muted icon, which says the sound is off. Until a read lands,
  // `unreadable` keeps the tile from stating it.
  let status = $state<AudioStatus>({ volume: 0, muted: false, output_type: "speaker" });
  let unreadable = $state(true);
  let writeTimer: ReturnType<typeof setTimeout> | null = null;

  onMount(() => {
    refresh();
    let stop: UnlistenFn | null = null;
    listen("arlen://audio-changed", refresh).then((u) => (stop = u));
    const interval = setInterval(refresh, 5_000);
    return () => {
      clearInterval(interval);
      stop?.();
      if (writeTimer) clearTimeout(writeTimer);
    };
  });

  async function refresh() {
    try {
      status = await invoke<AudioStatus>("get_audio_status");
      unreadable = false;
    } catch {}
  }

  function handleInput(value: number) {
    status.volume = value;
    if (writeTimer) clearTimeout(writeTimer);
    writeTimer = setTimeout(() => {
      invoke("set_audio_volume", { volume: Math.round(value) }).catch(() => {});
    }, 32);
  }

  /// Icon-button click toggles mute (the most likely "I want to do
  /// something fast"). Right-click on the tile opens the popover.
  async function toggleMute() {
    try {
      await invoke("toggle_audio_mute");
      await refresh();
    } catch {}
  }

  const sliderValue = $derived(status.muted ? 0 : status.volume);
  const subtitle = $derived(
    unreadable
      ? $t("sh.audio.unknown")
      : status.muted
        ? $t("sh.audio.muted")
        : status.output_type.includes("headphone")
          ? $t("sh.audio.headphones")
          : status.output_type.includes("hdmi")
            ? $t("sh.audio.hdmi")
            : $t("sh.audio.speakers"),
  );
</script>

<div class="audio-tile-wrap">
  <SliderTile
    label={$t("sh.tile.sound")}
    statusText={subtitle}
    valueKnown={!unreadable}
    value={sliderValue}
    min={0}
    max={100}
    oninput={handleInput}
    onDetail={() => openPopover("audio")}
    detailLabel="Open audio devices"
  >
    {#snippet icon()}
      <button
        type="button"
        class="audio-tile-icon-btn"
        onclick={(e) => {
          e.stopPropagation();
          toggleMute();
        }}
        title={$t("sh.tile.muteHint")}
      >
        {#if status.muted || status.volume === 0}
          <VolumeX size={16} strokeWidth={1.75} />
        {:else if status.output_type.includes("headphone")}
          <Headphones size={16} strokeWidth={1.75} />
        {:else if status.output_type === "speaker" && status.volume < 50}
          <Volume1 size={16} strokeWidth={1.75} />
        {:else if status.output_type.includes("hdmi") || status.output_type.includes("bluetooth_speaker")}
          <Speaker size={16} strokeWidth={1.75} />
        {:else}
          <Volume2 size={16} strokeWidth={1.75} />
        {/if}
      </button>
    {/snippet}
  </SliderTile>
</div>

<style>
  /* Wrapper sits inside a flex `.qs-grid-cell`, not directly in the
     CSS grid — `grid-column: span 2` would be a no-op here. The
     outer `.qs-grid-cell.full-row` already takes both columns; this
     wrapper just needs to fill that cell so the SliderTile inside
     stretches across the whole row. */
  .audio-tile-wrap {
    width: 100%;
  }
  .audio-tile-icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    color: var(--color-fg-shell);
    padding: 2px;
    border-radius: var(--radius-chip);
  }
  .audio-tile-icon-btn:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
  }
</style>
