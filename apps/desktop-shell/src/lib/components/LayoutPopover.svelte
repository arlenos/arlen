<script lang="ts">
  /// Layout popover: mode selection, gaps, smart gaps.

  import { activePopover, closePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { Separator } from "$lib/components/ui/separator/index.js";
  import { Layers, LayoutPanelLeft, Maximize } from "lucide-svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";

  interface LayoutState {
    mode: string;
    inner_gap: number;
    outer_gap: number;
    smart_gaps: boolean;
  }

  let state = $state<LayoutState>({
    mode: "floating",
    inner_gap: 8,
    outer_gap: 8,
    smart_gaps: true,
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
        <div class="gap-slider" style="--value: {(state.inner_gap / 24) * 100}%">
          <div class="gap-slider-track"></div>
          <div class="gap-slider-fill"></div>
          <div class="gap-slider-thumb"></div>
          <input
            type="range"
            min="0"
            max="24"
            value={state.inner_gap}
            oninput={(e) => setGap(parseInt(e.currentTarget.value))}
          />
        </div>
        <span class="gap-value">{state.inner_gap}px</span>
      </div>

      <!-- Smart Gaps -->
      <div class="toggle-row">
        <span class="toggle-label">Smart Gaps</span>
        <button
          class="smart-toggle"
          class:active={state.smart_gaps}
          onclick={toggleSmartGaps}
          role="switch"
          aria-checked={state.smart_gaps}
          aria-label="Smart Gaps"
        >
          <span class="smart-toggle-thumb"></span>
        </button>
      </div>

    </div>
  </div>
{/if}

<style>
  .pop-backdrop { position: fixed; inset: 0; z-index: 90; }
  .pop-panel {
    position: fixed; top: 40px; z-index: 100; border-radius: 10px;
    background: var(--color-bg-shell);
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    box-shadow: var(--shadow-lg);
    color: var(--color-fg-shell);
    display: flex; flex-direction: column;
    animation: pop-open 100ms ease-out both;
  }
  .pop-layout { right: 50px; width: 260px; }
  .pop-body { padding: 12px; display: flex; flex-direction: column; gap: 10px; }
  @keyframes pop-open { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }

  /* Mode pills */
  .mode-section { display: flex; flex-direction: column; gap: 6px; }
  .mode-pills { display: flex; gap: 4px; }
  .mode-pill {
    flex: 1; display: flex; flex-direction: column; align-items: center; gap: 4px;
    padding: 8px 4px; border-radius: 6px;
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
    background: color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    border-color: color-mix(in srgb, var(--color-fg-shell) 30%, transparent);
    color: var(--color-fg-shell);
  }

  /* Gap slider */
  .gap-row { display: flex; align-items: center; gap: 10px; }
  .gap-label { font-size: 0.75rem; flex-shrink: 0; }
  .gap-value { font-size: 0.6875rem; opacity: 0.5; min-width: 28px; text-align: right; }
  .gap-slider { position: relative; flex: 1; height: 20px; display: flex; align-items: center; }
  .gap-slider-track { position: absolute; left: 0; right: 0; height: 4px; background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent); border-radius: 2px; }
  .gap-slider-fill { position: absolute; left: 0; width: var(--value); height: 4px; background: color-mix(in srgb, var(--color-fg-shell) 60%, transparent); border-radius: 2px; }
  .gap-slider-thumb { position: absolute; left: var(--value); width: 14px; height: 14px; background: var(--color-fg-shell); border-radius: 50%; transform: translateX(-50%); box-shadow: var(--shadow-sm); pointer-events: none; }
  .gap-slider input[type="range"] { position: absolute; inset: 0; width: 100%; height: 100%; opacity: 0; cursor: pointer; margin: 0; appearance: none; -webkit-appearance: none; }

  /* Smart gaps toggle */
  .toggle-row { display: flex; align-items: center; justify-content: space-between; }
  .toggle-label { font-size: 0.75rem; }
  .smart-toggle {
    position: relative; width: 36px; height: 20px; border-radius: 10px;
    background: color-mix(in srgb, var(--color-fg-shell) 20%, transparent);
    border: none; cursor: pointer; padding: 0; flex-shrink: 0;
    transition: background-color 150ms ease;
  }
  .smart-toggle:hover { background: color-mix(in srgb, var(--color-fg-shell) 30%, transparent); }
  .smart-toggle.active { background: color-mix(in srgb, var(--color-fg-shell) 60%, transparent); }
  .smart-toggle-thumb {
    position: absolute; top: 2px; left: 2px;
    width: 16px; height: 16px; border-radius: 50%;
    background: var(--color-fg-shell);
    transition: transform 150ms ease;
  }
  .smart-toggle.active .smart-toggle-thumb { transform: translateX(16px); }

</style>
