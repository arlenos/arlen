<script lang="ts">
  /// The audio face (quickview-plan.md): a compact fixed window, no cover. The
  /// waveform is the file's face + scrubber; the title/artist from tags sit
  /// above it. Below, a centred transport cluster - prev / play-pause / next -
  /// with the play/pause as the large primary. Only the window controls
  /// auto-hide; the waveform and transport stay (you watch them while listening).
  /// `Space` toggles playback; prev/next move through the folder.
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { Play, Pause, Rewind, FastForward } from "@lucide/svelte";
  import Waveform from "./Waveform.svelte";
  import type { AudioMock } from "$lib/mock";

  let {
    file,
    onnext,
    onprev,
  }: {
    file: AudioMock;
    onnext?: () => void;
    onprev?: () => void;
  } = $props();

  let playing = $state(true);
  let progress = $state(0.33);
  let controlsVisible = $state(true);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  const fmt = (sec: number) => {
    const s = Math.max(0, Math.round(sec));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  };
  let elapsed = $derived(fmt(progress * file.durationSec));
  let total = $derived(fmt(file.durationSec));

  // Only the window controls fade on idle; the transport stays.
  function wake() {
    controlsVisible = true;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (controlsVisible = false), 2000);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === " ") {
      e.preventDefault();
      playing = !playing;
    } else if (e.key === "ArrowRight") onnext?.();
    else if (e.key === "ArrowLeft") onprev?.();
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="player" role="application" aria-label="Audio player" onpointermove={wake}>
  <div class="winctl" class:visible={controlsVisible}>
    <WindowButtons showMaximize={false} />
  </div>

  <div class="stack">
    <div class="title">
      {file.title}{#if file.artist}<span class="sep">·</span><span class="artist">{file.artist}</span>{/if}
    </div>

    <div class="wavebox">
      <Waveform peaks={file.peaks} {progress} onseek={(f) => (progress = f)} />
    </div>

    <div class="time">
      <span>{elapsed}</span><span>{total}</span>
    </div>

    <div class="transport">
      <Button
        variant="ghost"
        size="icon-lg"
        class="size-[48px]"
        aria-label="Previous file"
        onclick={() => onprev?.()}
      >
        <Rewind class="size-[22px]" strokeWidth={0} fill="currentColor" />
      </Button>
      <Button
        variant="secondary"
        size="icon-lg"
        class="size-[62px]"
        aria-label={playing ? "Pause" : "Play"}
        onclick={() => (playing = !playing)}
      >
        {#if playing}
          <Pause class="size-7" strokeWidth={0} fill="currentColor" />
        {:else}
          <Play class="size-7" strokeWidth={0} fill="currentColor" />
        {/if}
      </Button>
      <Button
        variant="ghost"
        size="icon-lg"
        class="size-[48px]"
        aria-label="Next file"
        onclick={() => onnext?.()}
      >
        <FastForward class="size-[22px]" strokeWidth={0} fill="currentColor" />
      </Button>
    </div>
  </div>
</div>

<style>
  .player {
    position: relative;
    width: 100%;
    height: 100%;
    background: var(--color-bg-app, #0f0f0f);
    color: var(--color-fg-primary, #fafafa);
    display: grid;
    place-items: center;
    padding: 0 30px;
    overflow: hidden;
    font-family: "Inter Variable", Inter, system-ui, sans-serif;
  }

  .winctl {
    position: absolute;
    top: 9px;
    right: 10px;
    opacity: 0;
    transition: opacity var(--duration-fast, 120ms) var(--easing-default, ease);
  }
  .winctl.visible {
    opacity: 1;
  }

  .stack {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .title {
    font-size: 14px;
    font-weight: 600;
    letter-spacing: -0.01em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .artist {
    color: var(--color-fg-secondary, #a1a1aa);
    font-weight: 500;
  }
  .sep {
    color: var(--color-fg-secondary, #a1a1aa);
    margin: 0 0.4em;
  }

  .wavebox {
    height: 70px;
  }

  .time {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--color-fg-secondary, #a1a1aa);
    font-variant-numeric: tabular-nums;
  }

  .transport {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 18px;
    margin-top: 4px;
  }
</style>
