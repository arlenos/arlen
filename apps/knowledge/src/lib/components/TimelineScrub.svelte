<script lang="ts">
  /// The time scrub (KA-R2, "drag the time control"): a thin rail above the
  /// spine with one tick per loaded day and a draggable grip carrying the day
  /// label. Dragging or clicking jumps the spine to that day; arrow keys step a
  /// day. This control NAVIGATES the recall spine - the as-of graph re-render
  /// (`valid_as_of`) is a later surface, so nothing here pretends to time-travel
  /// the graph itself.
  import { locale } from "$lib/i18n/messages";
  import { dayLabel, dayLabelShort, type TimelineDay } from "$lib/stores/timeline";
  import { t } from "$lib/i18n/messages";

  let {
    days,
    activeIndex = 0,
    onjump,
  }: {
    /// Newest first, mirroring the spine.
    days: TimelineDay[];
    /// Index into `days` of the day currently in view.
    activeIndex?: number;
    onjump: (index: number) => void;
  } = $props();

  // The rail runs oldest (left) to newest (right), so time reads forward.
  const count = $derived(days.length);
  const pos = $derived(count > 1 ? (count - 1 - activeIndex) / (count - 1) : 1);

  let railEl = $state<HTMLDivElement | null>(null);
  let dragging = $state(false);

  function indexAt(clientX: number): number {
    if (!railEl || count === 0) return 0;
    const r = railEl.getBoundingClientRect();
    const frac = Math.min(1, Math.max(0, (clientX - r.left) / r.width));
    const fromLeft = Math.round(frac * (count - 1));
    return count - 1 - fromLeft;
  }

  function onPointerDown(e: PointerEvent): void {
    dragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    onjump(indexAt(e.clientX));
  }
  function onPointerMove(e: PointerEvent): void {
    if (!dragging) return;
    onjump(indexAt(e.clientX));
  }
  function onPointerUp(): void {
    dragging = false;
  }

  function onKeydown(e: KeyboardEvent): void {
    // Left goes back in time (older = higher index), right comes forward.
    if (e.key === "ArrowLeft" && activeIndex < count - 1) {
      e.preventDefault();
      onjump(activeIndex + 1);
    } else if (e.key === "ArrowRight" && activeIndex > 0) {
      e.preventDefault();
      onjump(activeIndex - 1);
    }
  }
</script>

{#if count > 1}
  <div
    class="scrub"
    bind:this={railEl}
    role="slider"
    tabindex="0"
    aria-label={$t("k.tl.scrubAria")}
    aria-valuemin={0}
    aria-valuemax={count - 1}
    aria-valuenow={count - 1 - activeIndex}
    aria-valuetext={dayLabel(days[activeIndex].date, $locale)}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onkeydown={onKeydown}
  >
    <div class="rail"></div>
    {#each days as day, i (day.date)}
      <span class="tick" style={`left:${count > 1 ? ((count - 1 - i) / (count - 1)) * 100 : 100}%`} class:active={i === activeIndex}></span>
    {/each}
    <span class="grip" class:dragging style={`left:clamp(2.25rem, ${pos * 100}%, calc(100% - 2.25rem))`}>
      <span class="grip-label">{dayLabelShort(days[activeIndex].date, $locale)}</span>
    </span>
  </div>
{/if}

<style>
  .scrub {
    position: relative;
    height: 2.25rem;
    margin: 0.25rem 1.1rem 0;
    cursor: pointer;
    touch-action: none;
    outline: none;
  }
  .scrub:focus-visible .rail {
    background: color-mix(in srgb, var(--color-accent, var(--color-fg-primary)) 45%, transparent);
  }
  .rail {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 2px;
    transform: translateY(-50%);
    border-radius: 1px;
    background: color-mix(in srgb, var(--color-fg-primary) 14%, transparent);
  }
  .tick {
    position: absolute;
    top: 50%;
    width: 2px;
    height: 8px;
    transform: translate(-50%, -50%);
    border-radius: 1px;
    background: color-mix(in srgb, var(--color-fg-primary) 30%, transparent);
  }
  .tick.active {
    background: var(--color-fg-primary);
  }
  /* The grip carries the day it points at, so the control reads without a
     separate readout. */
  .grip {
    position: absolute;
    top: 50%;
    transform: translate(-50%, -50%);
    display: inline-flex;
    padding: 0.125rem 0.5rem;
    border: 1px solid color-mix(in srgb, var(--color-fg-primary) 16%, transparent);
    border-radius: var(--radius-chip, 4px);
    background: var(--color-bg-card);
    white-space: nowrap;
    user-select: none;
  }
  .grip.dragging {
    border-color: color-mix(in srgb, var(--color-fg-primary) 35%, transparent);
  }
  .grip-label {
    font-size: var(--text-2xs);
    font-weight: 600;
    color: color-mix(in srgb, var(--color-fg-primary) 75%, transparent);
  }
</style>
