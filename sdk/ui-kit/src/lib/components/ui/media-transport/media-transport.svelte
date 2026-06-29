<script lang="ts">
  /// The shared media transport cluster: previous / play-pause / next, on the
  /// kit `Button` (ghost sides, a secondary play at `lg`) with the filled
  /// Rewind / Play / Pause / FastForward glyphs. One component for every player
  /// surface - the QuickView audio face and the topbar Now-Playing applet share
  /// it, so the look (icons + the button corner radius) is one. `sm` is the
  /// compact topbar form; `lg` is the player/popup form.
  import { Button } from "../button";
  import { Play, Pause, SkipBack, SkipForward } from "@lucide/svelte";

  let {
    playing = false,
    canPrev = true,
    canNext = true,
    canControl = true,
    size = "lg",
    onprev,
    onplaypause,
    onnext,
  }: {
    playing?: boolean;
    canPrev?: boolean;
    canNext?: boolean;
    canControl?: boolean;
    size?: "sm" | "lg";
    onprev?: () => void;
    onplaypause?: () => void;
    onnext?: () => void;
  } = $props();

  const big = $derived(size === "lg");
  const btnSize = $derived(big ? "icon-lg" : "icon-xs");
  const sideBtn = $derived(big ? "size-[48px]" : "size-[26px]");
  const playBtn = $derived(big ? "size-[62px]" : "size-[26px]");
  const sideGlyph = $derived(big ? "size-[22px]" : "size-[15px]");
  const playGlyph = $derived(big ? "size-7" : "size-[15px]");
</script>

<div class="mt" class:lg={big}>
  <Button
    variant="ghost"
    size={btnSize}
    class={sideBtn}
    disabled={!canControl || !canPrev}
    aria-label="Previous"
    onclick={() => onprev?.()}
  >
    <SkipBack class={sideGlyph} strokeWidth={1.5} fill="currentColor" />
  </Button>
  <Button
    variant={big ? "secondary" : "ghost"}
    size={btnSize}
    class={playBtn}
    disabled={!canControl}
    aria-label={playing ? "Pause" : "Play"}
    onclick={() => onplaypause?.()}
  >
    {#if playing}
      <Pause class={playGlyph} strokeWidth={0} fill="currentColor" />
    {:else}
      <Play class={playGlyph} strokeWidth={0} fill="currentColor" />
    {/if}
  </Button>
  <Button
    variant="ghost"
    size={btnSize}
    class={sideBtn}
    disabled={!canControl || !canNext}
    aria-label="Next"
    onclick={() => onnext?.()}
  >
    <SkipForward class={sideGlyph} strokeWidth={1.5} fill="currentColor" />
  </Button>
</div>

<style>
  .mt {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 2px;
  }
  .mt.lg {
    gap: 18px;
  }
</style>
