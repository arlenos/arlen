<script lang="ts">
  /// The macOS-style floating-thumbnail handoff (screenshot-capture-plan.md §2):
  /// after a capture a thumbnail floats briefly in a corner. Ignore it and it
  /// auto-saves; click it to open the annotate surface. This resolves the tension
  /// between fast silent capture and inline annotation. Hovering pauses the
  /// auto-dismiss so the choice is never yanked away mid-decision.
  import { onMount } from "svelte";
  import { Pencil, Copy, Download, X } from "lucide-svelte";

  let {
    image,
    onAnnotate,
    onCopy,
    onSave,
    onDismiss,
    duration = 5000,
  }: {
    /// The captured image (the untouched base canvas).
    image: HTMLCanvasElement;
    onAnnotate?: () => void;
    onCopy?: () => void;
    onSave?: () => void;
    /// Fired on timeout or the dismiss button - the auto-save path.
    onDismiss?: () => void;
    duration?: number;
  } = $props();

  let src = $state("");
  let paused = $state(false);

  let timer: ReturnType<typeof setTimeout> | null = null;
  let remaining = 0;
  let startedAt = 0;

  function arm(ms: number) {
    startedAt = performance.now();
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => onDismiss?.(), ms);
  }
  function pause() {
    if (paused) return;
    paused = true;
    if (timer) clearTimeout(timer);
    remaining -= performance.now() - startedAt;
  }
  function resume() {
    if (!paused) return;
    paused = false;
    arm(remaining);
  }

  onMount(() => {
    src = image.toDataURL();
    remaining = duration;
    arm(duration);
    return () => {
      if (timer) clearTimeout(timer);
    };
  });

  function stop(e: Event) {
    e.stopPropagation();
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="thumb"
  role="button"
  tabindex="0"
  aria-label="Open the capture to annotate"
  onmouseenter={pause}
  onmouseleave={resume}
  onclick={() => onAnnotate?.()}
  onkeydown={(e) => (e.key === "Enter" || e.key === " ") && onAnnotate?.()}
>
  {#if src}<img class="thumb-img" {src} alt="Screen capture" />{/if}

  <div class="thumb-actions" role="group" aria-label="Capture actions">
    <button class="thumb-btn" title="Annotate" aria-label="Annotate" onclick={(e) => { stop(e); onAnnotate?.(); }}>
      <Pencil size={15} strokeWidth={2} />
    </button>
    <button class="thumb-btn" title="Copy" aria-label="Copy" onclick={(e) => { stop(e); onCopy?.(); }}>
      <Copy size={15} strokeWidth={2} />
    </button>
    <button class="thumb-btn" title="Save" aria-label="Save" onclick={(e) => { stop(e); onSave?.(); }}>
      <Download size={15} strokeWidth={2} />
    </button>
    <button class="thumb-btn" title="Dismiss" aria-label="Dismiss" onclick={(e) => { stop(e); onDismiss?.(); }}>
      <X size={15} strokeWidth={2} />
    </button>
  </div>

  <div class="thumb-timer">
    <div class="thumb-timer-fill" style={`animation-duration:${duration}ms; animation-play-state:${paused ? "paused" : "running"}`}></div>
  </div>
</div>

<style>
  .thumb {
    position: fixed;
    right: 1.25rem;
    bottom: 1.25rem;
    z-index: 60;
    width: 15rem;
    padding: 0.5rem;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-card);
    background: var(--color-bg-card);
    box-shadow: var(--shadow-lg, 0 12px 32px rgb(0 0 0 / 0.35));
    cursor: pointer;
    animation: thumb-in var(--duration-fast, 200ms) var(--ease-out, ease);
    --container-radius: var(--radius-card);
    --container-inset: 0.5rem;
  }
  @keyframes thumb-in {
    from {
      opacity: 0;
      transform: translateY(0.75rem) scale(0.98);
    }
  }
  .thumb-img {
    display: block;
    width: 100%;
    border-radius: max(0px, calc(var(--container-radius) - var(--container-inset)));
    /* A capture is content, not chrome: a hairline keeps it off the card. */
    border: 1px solid color-mix(in srgb, var(--foreground) 8%, transparent);
  }

  /* Quick actions ride the bottom of the image, revealed on hover. */
  .thumb-actions {
    position: absolute;
    left: 0.5rem;
    right: 0.5rem;
    bottom: 0.75rem;
    display: flex;
    justify-content: center;
    gap: 0.25rem;
    padding: 0.25rem;
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-bg-card) 82%, transparent);
    -webkit-backdrop-filter: blur(6px);
    backdrop-filter: blur(6px);
    opacity: 0;
    transform: translateY(0.25rem);
    transition:
      opacity var(--duration-fast, 150ms) var(--ease-out, ease),
      transform var(--duration-fast, 150ms) var(--ease-out, ease);
    pointer-events: none;
  }
  .thumb:hover .thumb-actions,
  .thumb:focus-visible .thumb-actions {
    opacity: 1;
    transform: translateY(0);
    pointer-events: auto;
  }
  .thumb-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    border: none;
    border-radius: var(--radius-input);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 70%, transparent);
    cursor: pointer;
    transition: background-color var(--duration-micro, 100ms) var(--ease-out, ease);
  }
  .thumb-btn:hover {
    background: color-mix(in srgb, var(--foreground) 12%, transparent);
    color: var(--foreground);
  }

  /* The auto-dismiss countdown: a thin bar that empties over the duration and
     freezes on hover (the timer pauses in lockstep). */
  .thumb-timer {
    height: 2px;
    margin-top: 0.4rem;
    border-radius: var(--radius-full, 9999px);
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
    overflow: hidden;
  }
  .thumb-timer-fill {
    height: 100%;
    width: 100%;
    border-radius: var(--radius-full, 9999px);
    background: color-mix(in srgb, var(--foreground) 40%, transparent);
    transform-origin: left;
    animation: thumb-drain linear forwards;
  }
  @keyframes thumb-drain {
    from {
      transform: scaleX(1);
    }
    to {
      transform: scaleX(0);
    }
  }
</style>
