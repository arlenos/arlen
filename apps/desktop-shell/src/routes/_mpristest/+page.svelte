<script lang="ts">
  /// Headless look-mock for the MPRIS "Now Playing" applet: the two inline
  /// transport variants (A/B) on a topbar strip over a wallpaper, plus the
  /// mini-player popup. Seeds the `nowPlaying` store (the live data is the
  /// coder's MPRIS client). `?state=noart` shows the app-icon/note fallback.
  /// Not shipped in any nav; a dev/test route to decide the A/B look with Tim.
  import { onMount } from "svelte";
  import MprisIndicator from "$lib/components/MprisIndicator.svelte";
  import MprisPopover from "$lib/components/MprisPopover.svelte";
  import { nowPlaying } from "$lib/stores/nowPlaying.js";

  const art = (h: number) =>
    `data:image/svg+xml;base64,${btoa(
      `<svg xmlns='http://www.w3.org/2000/svg' width='120' height='120'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='hsl(${h},58%,58%)'/><stop offset='1' stop-color='hsl(${(h + 50) % 360},55%,32%)'/></linearGradient></defs><rect width='120' height='120' fill='url(#g)'/><circle cx='78' cy='44' r='17' fill='hsl(${h},70%,84%)'/></svg>`,
    )}`;

  onMount(() => {
    const noart = new URLSearchParams(window.location.search).get("state") === "noart";
    nowPlaying.set({
      title: "Lateralus",
      artist: "Tool",
      album: "Lateralus",
      artUrl: noart ? null : art(280),
      status: "playing",
      position: 203,
      length: 562,
      canSeek: true,
      canPrev: true,
      canNext: true,
      canPause: true,
      canControl: true,
      activeId: "tool",
      players: [
        { id: "tool", app: "Spotify", icon: null, status: "playing" },
        { id: "browser", app: "Firefox", icon: noart ? null : art(120), status: "paused" },
        { id: "mpv", app: "mpv", icon: noart ? null : art(20), status: "paused" },
      ],
    });
  });
</script>

<div class="wallpaper">
  <div class="topbar">
    <span class="tb-spacer"></span>
    <MprisIndicator />
  </div>
  <p class="hint">Click an album-art thumb to open the mini-player.</p>
</div>

<MprisPopover />

<style>
  /* The shell tokens the applet reads, in case this dev route renders outside
     the themed shell root. */
  .wallpaper {
    --color-fg-shell: #e8e9ee;
    --color-accent: #83b3b1;
    min-height: 100vh;
    background:
      radial-gradient(120% 120% at 70% 10%, #3a4a6a 0%, #232a3f 45%, #15161d 100%);
    color: var(--color-fg-shell);
  }
  .topbar {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 36px;
    padding: 0 14px;
    background: color-mix(in srgb, #0c0d12 78%, transparent);
    backdrop-filter: blur(8px);
  }
  .tb-spacer {
    flex: 1;
  }
  .hint {
    padding: 16px;
    font-size: var(--text-xs);
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
  }
</style>
