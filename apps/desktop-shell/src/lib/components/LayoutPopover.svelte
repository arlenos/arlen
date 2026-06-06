<script lang="ts">
  /// Layout popover: mode selection, gaps, smart gaps.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { Separator } from "@lunaris/ui-kit/components/ui/separator/index.js";
  import { Layers, LayoutPanelLeft, Maximize } from "lucide-svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import Switch from "@lunaris/ui-kit/components/ui/switch/switch.svelte";
  import { FillSlider } from "@lunaris/ui-kit/components/ui/fill-slider";

  interface LayoutState {
    mode: string;
    inner_gap: number;
    outer_gap: number;
    smart_gaps: boolean;
    tiled_headers: boolean;
  }

  let state = $state<LayoutState>({
    mode: "floating",
    inner_gap: 8,
    outer_gap: 8,
    smart_gaps: true,
    tiled_headers: false,
  });

  let saveTimeout: ReturnType<typeof setTimeout> | null = null;

  async function poll() {
    try {
      state = await invoke<LayoutState>("get_layout_state");
    } catch {}
  }

  $effect(() => {
    if ($activePopover === "layout") poll();
  });

  // Clean up debounce timer on destroy.
  $effect(() => {
    return () => {
      if (saveTimeout) clearTimeout(saveTimeout);
    };
  });

  function setMode(mode: string) {
    state.mode = mode;
    invoke("set_layout_mode", { mode }).catch(() => {});
  }

  function setGap(value: number) {
    state.inner_gap = value;
    state.outer_gap = value;
    persistGaps();
  }

  function toggleSmartGaps() {
    state.smart_gaps = !state.smart_gaps;
    invoke("set_layout_smart_gaps", { enabled: state.smart_gaps }).catch(() => {});
  }

  function toggleTiledHeaders() {
    state.tiled_headers = !state.tiled_headers;
    invoke("set_layout_tiled_headers", { enabled: state.tiled_headers }).catch(() => {});
  }

  function persistGaps() {
    if (saveTimeout) clearTimeout(saveTimeout);
    saveTimeout = setTimeout(() => {
      invoke("set_layout_gaps", { inner: state.inner_gap, outer: state.outer_gap }).catch(() => {});
    }, 300);
  }
</script>

{#if $activePopover === "layout"}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-backdrop" onclick={closePopover}></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div class="pop-panel pop-layout shell-popover" onclick={(e) => e.stopPropagation()}>

    <PopoverHeader icon={LayoutPanelLeft} title="Layout" />

    <div class="pop-body">
      <!-- Mode Selector -->
      <div class="mode-section">
        <div class="mode-pills">
          <button
            class="mode-pill"
            class:active={state.mode === "floating"}
            onclick={() => setMode("floating")}
            title="Floating"
          >
            <Layers size={16} strokeWidth={1.5} />
            <span>Float</span>
          </button>
          <button
            class="mode-pill"
            class:active={state.mode === "tiling"}
            onclick={() => setMode("tiling")}
            title="Tiling"
          >
            <LayoutPanelLeft size={16} strokeWidth={1.5} />
            <span>Tile</span>
          </button>
          <button
            class="mode-pill"
            class:active={state.mode === "monocle"}
            onclick={() => setMode("monocle")}
            title="Monocle"
          >
            <Maximize size={16} strokeWidth={1.5} />
            <span>Mono</span>
          </button>
        </div>
      </div>

      <Separator class="opacity-10" />

      <!-- Gaps -->
      <div class="gap-row">
        <span class="gap-label">Gaps</span>
        <div class="gap-slider-wrap">
          <FillSlider
            value={state.inner_gap}
            min={0}
            max={24}
            step={1}
            size="sm"
            ariaLabel="Inner gap"
            oninput={(v) => setGap(v)}
          />
        </div>
        <span class="gap-value">{state.inner_gap}px</span>
      </div>

      <!-- Smart Gaps -->
      <div class="toggle-row">
        <span class="toggle-label">Smart Gaps</span>
        <Switch
          value={state.smart_gaps}
          onchange={toggleSmartGaps}
          ariaLabel="Smart Gaps"
        />
      </div>

      <!--
        Tiled Headers: only meaningful when tiled windows actually exist.
        Hidden in floating mode to keep the UI focused. The setting is
        global (compositor.toml [layout]) so toggling it in tiling/monocle
        and switching back to floating preserves the value silently.
      -->
      {#if state.mode === "tiling" || state.mode === "monocle"}
        <div class="toggle-row" title="Hide compositor title bars on tiled windows">
          <span class="toggle-label">Tiled Headers</span>
          <Switch
            value={state.tiled_headers}
            onchange={toggleTiledHeaders}
            ariaLabel="Tiled Headers"
          />
        </div>
      {/if}

    </div>
  </div>
{/if}

<style>
  .pop-backdrop { position: fixed; inset: 0; z-index: 90; }
  .pop-panel {
    position: fixed; top: 40px; z-index: 100; border-radius: var(--radius-card);
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-shell);
    display: flex; flex-direction: column;
    animation: lunaris-popover-in var(--duration-medium) var(--ease-out) both;
    transform-origin: top center;
  }
  .pop-layout { right: 50px; width: 260px; }
  .pop-body { padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  /* Entry keyframes defined in sdk/ui-kit/src/lib/motion.css. */

  /* Mode pills */
  .mode-section { display: flex; flex-direction: column; gap: 6px; }
  .mode-pills { display: flex; gap: 4px; }
  .mode-pill {
    flex: 1; display: flex; flex-direction: column; align-items: center; gap: 4px;
    padding: 8px 4px; border-radius: var(--radius-input);
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    cursor: pointer; font-size: 0.625rem; font-weight: 500;
    transition: all 100ms ease;
  }
  .mode-pill:hover {
    background: color-mix(in srgb, var(--color-fg-shell) 10%, transparent);
    color: var(--color-fg-shell);
  }
  .mode-pill.active {
    background: color-mix(in srgb, var(--color-accent) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-accent) 30%, transparent);
    color: var(--color-fg-shell);
  }

  /* Gap slider */
  .gap-row { display: flex; align-items: center; gap: 10px; }
  .gap-label { font-size: 0.75rem; flex-shrink: 0; }
  .gap-value { font-size: 0.6875rem; opacity: 0.5; min-width: 28px; text-align: right; }
  .gap-slider-wrap { flex: 1; display: flex; align-items: center; }

  /* Toggle row uses the same flex+gap pattern as `.gap-row` so
     the rhythm of "label · control" reads consistently across
     rows. `space-between` looks fine in isolation but creates a
     visible right-edge jitter when sibling rows have value pills
     pinned right. */
  .toggle-row { display: flex; align-items: center; gap: 10px; }
  .toggle-label {
    flex: 1;
    min-width: 0;
    font-size: 0.75rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

</style>
