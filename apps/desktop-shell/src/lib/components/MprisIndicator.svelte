<script lang="ts">
  /// The inline "Now Playing" applet: an album-art tile (click → the mini-player
  /// popup) plus one small play/pause, in the topbar-applet register with a
  /// unified hover over the whole element. A composite (two clickables - art →
  /// popover, play/pause → control), so it does not wrap the single-trigger
  /// `Applet`, but matches its register tokens + hover. Full transport lives in
  /// the popup. A count badge marks more than one registered player. Hidden when
  /// nothing is registered.
  import { Music, Play, Pause } from "lucide-svelte";
  import { activePopover, togglePopover, hoverPopover } from "$lib/stores/activePopover.js";
  import { nowPlaying, playPause } from "$lib/stores/nowPlaying.js";

  const isOpen = $derived($activePopover === "mpris");
  const tip = $derived($nowPlaying ? `${$nowPlaying.title} · ${$nowPlaying.artist}` : "");
</script>

{#if $nowPlaying}
  {@const n = $nowPlaying}
  {@const playing = n.status === "playing"}
  <div
    class="mpris"
    class:open={isOpen}
    onmouseenter={() => hoverPopover("mpris")}
    role="group"
    aria-label="Now playing"
  >
    <button
      class="mpris-art"
      title={tip}
      aria-label="Open the player"
      onclick={() => togglePopover("mpris")}
    >
      {#if n.artUrl}
        <img src={n.artUrl} alt="" draggable="false" />
      {:else}
        <Music size={11} strokeWidth={1.75} />
      {/if}
    </button>

    <button
      class="mpris-play"
      aria-label={playing ? "Pause" : "Play"}
      disabled={!n.canControl}
      onclick={() => playPause()}
    >
      {#if playing}
        <Pause size={14} strokeWidth={0} fill="currentColor" />
      {:else}
        <Play size={14} strokeWidth={0} fill="currentColor" />
      {/if}
    </button>
  </div>
{/if}

<style>
  /* The whole element is one applet: a unified hover wash + the accent-open
     tint, on the topbar-applet register tokens so it matches its neighbours. */
  .mpris {
    display: inline-flex;
    align-items: center;
    gap: 1px;
    height: var(--topbar-applet-h, 28px);
    padding: 0 3px;
    border-radius: var(--topbar-applet-radius, var(--radius-chip));
    transition: background-color var(--duration-fast, 100ms) var(--ease-out, ease);
  }
  .mpris:hover {
    background: var(
      --topbar-applet-hover-bg,
      color-mix(in srgb, var(--color-fg-shell) 10%, transparent)
    );
  }
  .mpris.open {
    background: color-mix(in srgb, var(--color-accent) 15%, transparent);
  }

  .mpris-art {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--color-fg-shell) 14%, transparent);
    color: color-mix(in srgb, var(--color-fg-shell) 60%, transparent);
  }
  .mpris-art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    border-radius: var(--radius-chip);
  }
  .mpris.open .mpris-art {
    color: var(--color-accent);
  }

  .mpris-play {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: var(--radius-chip);
    color: color-mix(in srgb, var(--color-fg-shell) 78%, transparent);
    transition:
      background-color var(--duration-fast, 100ms) var(--ease-out, ease),
      color var(--duration-fast, 100ms) var(--ease-out, ease),
      opacity var(--duration-fast, 100ms) var(--ease-out, ease);
  }
  .mpris-play:hover:not(:disabled) {
    background: color-mix(in srgb, var(--color-fg-shell) 16%, transparent);
    color: var(--color-fg-shell);
  }
  .mpris-play:disabled {
    opacity: 0.35;
  }
</style>
