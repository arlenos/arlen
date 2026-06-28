<script lang="ts">
  /// The right pane: the full render of a large text/data artifact (the chat card
  /// opens it; a freshly produced one opens it automatically). Header is just the
  /// title, kind badge and close - no action toolbar; actions live in the
  /// right-click context menu (Copy live; Save to file / Pin are coder seams,
  /// disabled until wired). The body fills the pane and scrolls; the left edge
  /// drags to resize (clamped to a minimum width).
  import { onDestroy } from "svelte";
  import { X } from "@lucide/svelte";
  import * as ContextMenu from "@arlen/ui-kit/components/ui/context-menu";
  import ArtifactView from "$lib/components/artifact/ArtifactView.svelte";
  import { kindLabel, type Artifact } from "$lib/components/artifact/types";

  let {
    artifact,
    onclose,
    onsave,
    onpin,
  }: { artifact: Artifact; onclose?: () => void; onsave?: () => void; onpin?: () => void } = $props();

  const title = $derived(artifact.meta.title ?? kindLabel(artifact.kind));

  // Drag the left edge to resize, clamped to a sensible band. Window listeners
  // (not pointer capture) so the drag keeps tracking once the cursor leaves the
  // thin handle, the robust resize-handle pattern.
  const MIN = 320;
  const MAX = 880;
  let width = $state(420);
  let startX = 0;
  let startW = 0;
  const clamp = (w: number) => Math.max(MIN, Math.min(MAX, w));
  function onMove(e: PointerEvent) {
    width = clamp(startW + (startX - e.clientX));
  }
  function onUp() {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
  }
  function onDown(e: PointerEvent) {
    e.preventDefault();
    startX = e.clientX;
    startW = width;
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowLeft") width = clamp(width + 16);
    else if (e.key === "ArrowRight") width = clamp(width - 16);
  }
  onDestroy(() => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
  });

  async function copy() {
    try {
      await navigator.clipboard.writeText(artifact.text);
    } catch {
      // clipboard may be unavailable
    }
  }
</script>

<aside class="ap" style="width:{width}px" aria-label="Artifact">
  <div
    class="ap-resize"
    role="separator"
    aria-orientation="vertical"
    aria-label="Resize artifact pane"
    tabindex="0"
    onpointerdown={onDown}
    onkeydown={onKey}
  ></div>
  <header class="ap-head">
    <span class="ap-title" title={title}>{title}</span>
    <span class="ap-badge">{kindLabel(artifact.kind)}</span>
    <button class="ap-close" aria-label="Close" onclick={() => onclose?.()}>
      <X size={15} strokeWidth={2} />
    </button>
  </header>
  <ContextMenu.Root>
    <ContextMenu.Trigger class="ap-body-trigger">
      <div class="ap-body"><ArtifactView {artifact} /></div>
    </ContextMenu.Trigger>
    <ContextMenu.Content class="w-52">
      <ContextMenu.Item onclick={copy}>Copy</ContextMenu.Item>
      <ContextMenu.Item onclick={onsave} disabled={!onsave}>Save to file&hellip;</ContextMenu.Item>
      <ContextMenu.Item onclick={onpin} disabled={!onpin}>Pin</ContextMenu.Item>
    </ContextMenu.Content>
  </ContextMenu.Root>
</aside>

<style>
  .ap {
    position: relative;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-left: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
    overflow: hidden;
  }
  /* The drag strip sits over the left border. */
  .ap-resize {
    position: absolute;
    left: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    z-index: 2;
    cursor: ew-resize;
    background: transparent;
  }
  .ap-resize:hover,
  .ap-resize:focus-visible {
    background: color-mix(in srgb, var(--color-accent) 35%, transparent);
    outline: none;
  }
  .ap-head {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.625rem 0.875rem;
    border-bottom: 1px solid color-mix(in srgb, var(--foreground) 7%, transparent);
  }
  .ap-title {
    flex: 1;
    min-width: 0;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--foreground);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ap-badge {
    flex-shrink: 0;
    padding: 0.0625rem 0.375rem;
    border-radius: var(--radius-chip);
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    font-size: 0.625rem;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: color-mix(in srgb, var(--foreground) 60%, transparent);
  }
  .ap-close {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--height-control-compact, 24px);
    height: var(--height-control-compact, 24px);
    border: none;
    border-radius: var(--radius-chip);
    background: transparent;
    color: color-mix(in srgb, var(--foreground) 55%, transparent);
  }
  .ap-close:hover {
    background: color-mix(in srgb, var(--foreground) 8%, transparent);
    color: var(--foreground);
  }
  :global(.ap-body-trigger) {
    display: block;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .ap-body {
    /* The focused view: code reads larger + roomier, and fills the height (the
       pane body owns the scroll) rather than the inline 24rem cap. */
    --artifact-code-size: 0.875rem;
    --artifact-max-height: none;
    padding: 1rem;
  }
</style>
