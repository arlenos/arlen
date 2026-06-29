<script lang="ts">
  /// The "Now Playing" mini-player popup: a hero (album art + title/artist), the
  /// transport, an interpolated scrubber, and a thin switcher of the other
  /// players. One control surface for every MPRIS producer. No volume (the
  /// system owns output level); no remote-art auto-fetch (the app-icon fallback).
  import { Music } from "lucide-svelte";
  import { FillSlider } from "@arlen/ui-kit/components/ui/fill-slider";
  import { MediaTransport } from "@arlen/ui-kit/components/ui/media-transport";
  import ShellPopover from "$lib/components/shared/ShellPopover.svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import { togglePopover } from "$lib/stores/activePopover.js";
  import {
    nowPlaying,
    otherPlayers,
    playPause,
    previous,
    next,
    seek,
    pinPlayer,
  } from "$lib/stores/nowPlaying.js";

  // The header names the active source (the multi-source context), not a generic
  // "Now Playing"; the gear bridges to the sound controls the player omits.
  const activeApp = $derived(
    $nowPlaying?.players.find((p) => p.id === $nowPlaying?.activeId)?.app ?? "Player",
  );

  // mm:ss for the scrubber labels.
  function clock(seconds: number): string {
    const s = Math.max(0, Math.floor(seconds));
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
  }
</script>

<ShellPopover id="mpris" width={304} right={120} bodyPadding="0" bodyGap="0">
  {#snippet header()}
    <PopoverHeader icon={Music} title={activeApp} onSettings={() => togglePopover("audio")} />
  {/snippet}

  {#if $nowPlaying}
    {@const n = $nowPlaying}
    {@const playing = n.status === "playing"}
    <div class="np">
      <div class="np-hero">
        <span class="np-art">
          {#if n.artUrl}
            <img src={n.artUrl} alt="" draggable="false" />
          {:else}
            <Music size={26} strokeWidth={1.25} />
          {/if}
        </span>
        <div class="np-meta">
          <span class="np-title" title={n.title}>{n.title}</span>
          <span class="np-artist" title={n.artist}>{n.artist}</span>
          {#if n.album}<span class="np-album" title={n.album}>{n.album}</span>{/if}
        </div>
      </div>

      {#if n.canSeek}
        <div class="np-scrub">
          <FillSlider
            value={n.position}
            min={0}
            max={Math.max(1, n.length)}
            step={1}
            size="sm"
            ariaLabel="Seek"
            oninput={(v) => seek(v)}
          />
          <div class="np-times">
            <span>{clock(n.position)}</span>
            <span>{clock(n.length)}</span>
          </div>
        </div>
      {/if}

      <MediaTransport
        size="lg"
        {playing}
        canPrev={n.canPrev}
        canNext={n.canNext}
        canControl={n.canControl}
        onprev={() => previous()}
        onplaypause={() => playPause()}
        onnext={() => next()}
      />

      {#if $otherPlayers.length > 0}
        <div class="np-switch">
          {#each $otherPlayers as p (p.id)}
            <button
              class="np-player"
              class:paused={p.status !== "playing"}
              title={p.app}
              onclick={() => pinPlayer(p.id)}
            >
              <span class="np-player-icon">
                {#if p.icon}
                  <img src={p.icon} alt="" />
                {:else}
                  <Music size={12} strokeWidth={1.75} />
                {/if}
              </span>
              <span class="np-player-name">{p.app}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</ShellPopover>

<style>
  .np {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px;
  }
  .np-hero {
    display: flex;
    align-items: center;
    gap: 12px;
    min-width: 0;
  }
  .np-art {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    flex-shrink: 0;
    border-radius: var(--radius-card, 8px);
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: color-mix(in srgb, var(--color-fg-shell) 55%, transparent);
    overflow: hidden;
  }
  .np-art img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .np-meta {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }
  .np-title {
    font-size: 0.875rem;
    font-weight: 600;
    color: var(--color-fg-shell);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .np-artist {
    font-size: 0.8125rem;
    color: color-mix(in srgb, var(--color-fg-shell) 70%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .np-album {
    font-size: 0.6875rem;
    color: color-mix(in srgb, var(--color-fg-shell) 45%, transparent);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .np-scrub {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .np-times {
    display: flex;
    justify-content: space-between;
    font-size: 0.6875rem;
    font-variant-numeric: tabular-nums;
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
  }

  .np-switch {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding-top: 10px;
    border-top: 1px solid color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
  }
  .np-player {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 9px 0 5px;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    border-radius: var(--radius-chip);
    background: transparent;
    color: color-mix(in srgb, var(--color-fg-shell) 75%, transparent);
    font-size: 0.75rem;
  }
  .np-player:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 8%, transparent);
    color: var(--color-fg-shell);
  }
  .np-player.paused {
    opacity: 0.55;
  }
  .np-player-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
  }
  .np-player-icon img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    border-radius: 3px;
  }
  .np-player-name {
    max-width: 8rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
