<script lang="ts">
  /// The video face (quickview-plan.md): the frame fills the window edge-to-edge
  /// like the image, plus one auto-hide bottom dock holding the progress slider
  /// (the kit `FillSlider`, click/drag/keys = seek) over a `Toolbar` control row:
  /// play/pause, elapsed/total, name + position, fullscreen. A large centre play
  /// shows only while paused. Audio-track + subtitles live in the menu, not the
  /// dock. prev/next edge arrows; the top strip drags the frameless window.
  /// `Space` toggles playback, `F` fullscreen. Decoded frames are the coder's.
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { Button } from "@arlen/ui-kit/components/ui/button";
  import { FillSlider } from "@arlen/ui-kit/components/ui/fill-slider";
  import { Toolbar } from "@arlen/ui-kit/components/ui/toolbar";
  import {
    Play,
    Pause,
    ChevronLeft,
    ChevronRight,
    Maximize,
  } from "@lucide/svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { tauriAvailable } from "$lib/tauri";
  import type { VideoMock } from "$lib/mock";

  let {
    file,
    onnext,
    onprev,
  }: {
    file: VideoMock;
    onnext?: () => void;
    onprev?: () => void;
  } = $props();

  let playing = $state(true);
  let progress = $state(0.3);
  let chromeVisible = $state(true);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  const fmt = (sec: number) => {
    const s = Math.max(0, Math.round(sec));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  };
  let elapsed = $derived(fmt(progress * file.durationSec));
  let total = $derived(fmt(file.durationSec));

  function wake() {
    chromeVisible = true;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (chromeVisible = false), 2000);
  }

  async function startDrag(e: PointerEvent) {
    if (!tauriAvailable || e.button !== 0) return;
    if ((e.target as HTMLElement)?.closest("button")) return;
    await getCurrentWindow().startDragging();
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

<div
  class="viewer"
  class:chrome={chromeVisible}
  role="application"
  aria-label="Video player"
  onpointermove={wake}
>
  <!-- The frame fills the window. A gradient still stands in for decoded video. -->
  <div class="frame"></div>

  <div class="scrim top"></div>
  <div class="scrim bottom"></div>

  <!-- The phantom-titlebar strip: drag the frameless window from here. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="dragstrip" onpointerdown={startDrag}></div>

  <div class="winctl">
    <WindowButtons showMaximize={false} />
  </div>

  {#if !playing}
    <button class="centerplay" aria-label="Play" onclick={() => (playing = true)}>
      <Play class="size-7" strokeWidth={1.5} fill="currentColor" />
    </button>
  {/if}

  <button class="edge left" aria-label="Previous file" onclick={() => onprev?.()}>
    <ChevronLeft size={30} strokeWidth={2} />
  </button>
  <button class="edge right" aria-label="Next file" onclick={() => onnext?.()}>
    <ChevronRight size={30} strokeWidth={2} />
  </button>

  <div class="dock">
    <FillSlider
      value={progress * 100}
      size="sm"
      ariaLabel="Seek"
      oninput={(v) => (progress = v / 100)}
    />

    <Toolbar>
      {#snippet start()}
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={playing ? "Pause" : "Play"}
          onclick={() => (playing = !playing)}
        >
          {#if playing}
            <Pause class="size-[17px]" strokeWidth={1.5} fill="currentColor" />
          {:else}
            <Play class="size-[17px]" strokeWidth={1.5} fill="currentColor" />
          {/if}
        </Button>
        <span class="time">{elapsed} / {total}</span>
      {/snippet}
      {#snippet end()}
        <span class="meta">{file.name}<span class="dot">·</span>{file.index} / {file.total}</span>
        <Button variant="ghost" size="icon-sm" aria-label="Fullscreen">
          <Maximize class="size-[16px]" strokeWidth={2} />
        </Button>
      {/snippet}
    </Toolbar>
  </div>
</div>

<style>
  .viewer {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: #0a0a0a;
    font-family: "Inter Variable", Inter, system-ui, sans-serif;
    color: var(--color-fg-primary, #fafafa);
  }

  .frame {
    position: absolute;
    inset: 0;
    background:
      radial-gradient(120% 90% at 50% 18%, rgba(80, 110, 150, 0.35), transparent 60%),
      linear-gradient(180deg, #10161f 0%, #1a2230 45%, #20160f 78%, #0c0a08 100%);
  }

  .scrim,
  .winctl,
  .edge,
  .dock {
    opacity: 0;
    transition: opacity var(--duration-fast, 120ms) var(--easing-default, ease);
    pointer-events: none;
  }
  .viewer.chrome .scrim,
  .viewer.chrome .winctl,
  .viewer.chrome .edge,
  .viewer.chrome .dock {
    opacity: 1;
  }
  .viewer.chrome .winctl,
  .viewer.chrome .edge,
  .viewer.chrome .dock {
    pointer-events: auto;
  }

  .scrim {
    position: absolute;
    left: 0;
    right: 0;
    height: 96px;
  }
  .scrim.top {
    top: 0;
    background: linear-gradient(180deg, rgba(0, 0, 0, 0.4), transparent);
  }
  .scrim.bottom {
    bottom: 0;
    background: linear-gradient(0deg, rgba(0, 0, 0, 0.52), transparent);
  }

  /* Invisible drag region where a titlebar would be (always grabbable). */
  .dragstrip {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: var(--height-bar, 36px);
  }

  .winctl {
    position: absolute;
    top: 9px;
    right: 11px;
    z-index: 2;
  }

  .centerplay {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 66px;
    height: 66px;
    display: grid;
    place-items: center;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary, #fafafa) 22%, transparent);
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, #0a0a0a 55%, transparent);
    color: var(--color-fg-primary, #fafafa);
    cursor: pointer;
    backdrop-filter: blur(8px);
  }

  .edge {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 46px;
    height: 80px;
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    color: var(--color-fg-primary, #fafafa);
    cursor: pointer;
    filter: drop-shadow(0 1px 3px rgba(0, 0, 0, 0.5));
  }
  .edge.left {
    left: 8px;
  }
  .edge.right {
    right: 8px;
  }

  /* One contained bottom dock (same pill as the image face): the scrubber over
     the control row, on its own surface, not loose on the scrim. */
  .dock {
    position: absolute;
    left: 14px;
    right: 14px;
    bottom: 14px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 9px 12px 7px;
    border-radius: var(--radius-card, 12px);
    background: color-mix(in srgb, #141414 80%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-fg-primary, #fafafa) 12%, transparent);
    box-shadow: 0 8px 26px rgba(0, 0, 0, 0.4);
    backdrop-filter: blur(12px);
  }

  .time {
    font-size: 12px;
    color: var(--color-fg-primary, #fafafa);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .meta {
    font-size: 12px;
    color: var(--color-fg-secondary, #a1a1aa);
    white-space: nowrap;
  }
  .dot {
    color: var(--color-fg-secondary, #a1a1aa);
    margin: 0 0.4em;
  }
</style>
