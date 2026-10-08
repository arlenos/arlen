<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// The topbar workspace strip. Three densities, picked by the
  /// host from the workspace count: pills (≤5), dots (≤9), and a
  /// plain "n / total" text readout beyond that.
  ///
  /// Motion (design-system §6b): the act is switching, so in the pill row the
  /// active workspace is one highlight that slides from the old pill to the
  /// new one, the same gesture as the kit's segmented control; in the dot
  /// row the active dot grows in place. Both on `--duration-normal`, which the
  /// theme zeroes under reduce motion. A press is the kit Button's press.
  ///
  /// `peekIndex` borrows the same motion to answer a refused move: while it is
  /// set, the highlight goes to that workspace instead of the active one, and
  /// clearing it brings the highlight back. No second animation, no text.
  import { tick } from "svelte";

  import type { WorkspaceInfo } from "$lib/stores/workspaces.js";
  import { pillLabel, fullLabel } from "$lib/workspace/format.js";

  let {
    workspaces,
    mode,
    activeIndex,
    peekIndex = null,
    onActivate,
  }: {
    workspaces: WorkspaceInfo[];
    mode: "pills" | "dots" | "text";
    /// Index of the active workspace, -1 when none is flagged.
    activeIndex: number;
    /// A workspace to show in place of the active one for a moment, or null.
    peekIndex?: number | null;
    onActivate: (id: string) => void;
  } = $props();

  // The sliding highlight in the pill row, measured from the active pill.
  let pills = $state<HTMLButtonElement[]>([]);
  let mark = $state<{ x: number; w: number } | null>(null);
  let placed = $state(false);

  /// Which workspace the highlight is on: the peeked one while there is one.
  const shownIndex = $derived(peekIndex ?? workspaces.findIndex((w) => w.active));

  function measure(): void {
    const i = shownIndex;
    const el = i >= 0 ? pills[i] : null;
    mark = el ? { x: el.offsetLeft, w: el.offsetWidth } : null;
  }

  $effect(() => {
    void workspaces;
    void mode;
    void shownIndex;
    tick().then(() => {
      measure();
      if (!placed && mark) requestAnimationFrame(() => (placed = true));
    });
  });
</script>

{#if mode === "pills"}
  <div class="indicator pills" role="group" aria-label={$t("sh.ws.workspaces")}>
    {#if mark}
      <span class="pill-mark" class:placed aria-hidden="true" style="transform: translateX({mark.x}px); width: {mark.w}px"></span>
    {/if}
    {#each workspaces as ws, i (ws.id)}
      <button
        bind:this={pills[i]}
        class="pill"
        class:pill-active={ws.active}
        onclick={() => onActivate(ws.id)}
        aria-label={fullLabel(ws, i)}
        aria-pressed={ws.active}
      >
        {pillLabel(ws, i)}
      </button>
    {/each}
  </div>
{:else if mode === "dots"}
  <div class="indicator" role="group" aria-label={$t("sh.ws.workspaces")}>
    {#each workspaces as ws, i (ws.id)}
      <button
        class="dot-btn"
        onclick={() => onActivate(ws.id)}
        aria-label={fullLabel(ws, i)}
        aria-pressed={ws.active}
      >
        <span class="dot" class:dot-active={i === shownIndex}></span>
      </button>
    {/each}
  </div>
{:else}
  <div class="indicator" role="group" aria-label={$t("sh.ws.workspaces")}>
    <span class="ws-text">
      {(peekIndex ?? activeIndex) >= 0 ? (peekIndex ?? activeIndex) + 1 : 1} / {workspaces.length}
    </span>
  </div>
{/if}

<style>
  .indicator {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  /* ── Pills ──────────────────────────────────────────────────────────── */

  .pill {
    display: flex;
    align-items: center;
    justify-content: center;
    height: var(--height-control-compact, 26px);
    min-width: 32px;
    padding: 0 10px;
    border-radius: var(--radius-card);
    border: none;
    font-size: var(--text-2xs);
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
    position: relative;
    transition:
      background-color var(--duration-fast, 150ms) ease,
      color var(--duration-normal, 250ms) var(--ease-out),
      transform var(--duration-micro, 100ms) ease;
    background: transparent;
    color: var(--foreground);
  }

  .indicator.pills {
    position: relative;
  }

  /* One highlight for the active workspace, moved rather than redrawn. */
  .pill-mark {
    position: absolute;
    top: 50%;
    left: 0;
    height: var(--height-control-compact, 26px);
    margin-top: calc(var(--height-control-compact, 26px) / -2);
    border-radius: var(--radius-card);
    background: color-mix(in srgb, var(--color-accent) 18%, transparent);
    pointer-events: none;
  }
  .pill-mark.placed {
    transition:
      transform var(--duration-normal, 250ms) var(--ease-out),
      width var(--duration-normal, 250ms) var(--ease-out);
  }

  .pill:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
  }

  .pill:active {
    transform: scale(0.95);
  }

  .pill-active {
    color: var(--color-accent);
  }

  /* The highlight under it carries the tint; hover only deepens it a little. */
  .pill-active:hover {
    background: color-mix(in srgb, var(--color-accent) 8%, transparent);
  }

  /* ── Dots ───────────────────────────────────────────────────────────── */

  .dot-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 26px);
    height: var(--height-control-compact, 26px);
    padding: 0;
    border: none;
    background: transparent;
    border-radius: var(--radius-full);
    transition: transform var(--duration-micro, 100ms) ease;
  }

  .dot-btn:active {
    transform: scale(0.85);
  }

  .dot {
    display: block;
    width: 5px;
    height: 5px;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 45%, transparent);
    transition:
      width var(--duration-normal, 250ms) var(--ease-out),
      height var(--duration-normal, 250ms) var(--ease-out),
      background-color var(--duration-normal, 250ms) var(--ease-out);
  }

  .dot-btn:hover .dot {
    background: color-mix(in srgb, var(--foreground) 70%, transparent);
  }

  .dot-active {
    width: 7px;
    height: 7px;
    background: var(--color-accent);
  }

  .dot-btn:hover .dot-active {
    background: color-mix(
      in srgb,
      var(--color-accent) 85%,
      var(--color-fg-shell) 15%
    );
  }

  /* ── Text ───────────────────────────────────────────────────────────── */

  .ws-text {
    font-size: var(--text-2xs);
    font-weight: 500;
    color: var(--foreground);
    letter-spacing: 0.02em;
  }
</style>
