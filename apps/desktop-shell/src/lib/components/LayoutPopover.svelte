<script lang="ts">
  import { t } from "$lib/i18n/messages";
  /// Layout popover: mode selection, gaps, smart gaps.

  import { activePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { Separator } from "@arlen/ui-kit/components/ui/separator/index.js";
  import { Layers, LayoutPanelLeft, Maximize } from "lucide-svelte";
  import ShellPopover from "$lib/components/shared/ShellPopover.svelte";
  import PopoverHeader from "$lib/components/shared/PopoverHeader.svelte";
  import PopoverErrorBanner from "$lib/components/shared/PopoverErrorBanner.svelte";
  import Switch from "@arlen/ui-kit/components/ui/switch/switch.svelte";
  import { FillSlider } from "@arlen/ui-kit/components/ui/fill-slider";

  interface LayoutState {
    mode: string;
    inner_gap: number;
    outer_gap: number;
    smart_gaps: boolean;
    tiled_headers: boolean;
  }

  let layout = $state<LayoutState>({
    mode: "floating",
    inner_gap: 8,
    outer_gap: 8,
    smart_gaps: true,
    tiled_headers: false,
  });

  let saveTimeout: ReturnType<typeof setTimeout> | null = null;
  /// Whether the layout has been READ. Every field above has a plausible default
  /// - floating, 8px gaps, smart gaps on - so a compositor that did not answer
  /// rendered a complete description of the window layout, with "Floating"
  /// showing as the selected mode. All of it invented.
  let read = $state(false);

  async function poll() {
    try {
      layout = await invoke<LayoutState>("get_layout_state");
      read = true;
    } catch {
      read = false;
    }
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

  /// Apply optimistically and put it back if the compositor refuses. Nothing
  /// re-polls this panel after a write, so a swallowed refusal left the pill
  /// showing a mode the windows were never in - not for three seconds, but until
  /// the panel is reopened.
  function setMode(mode: string) {
    const previous = layout.mode;
    layout.mode = mode;
    invoke("set_layout_mode", { mode }).catch(() => {
      layout.mode = previous;
    });
  }

  function setGap(value: number) {
    layout.inner_gap = value;
    layout.outer_gap = value;
    persistGaps();
  }

  function toggleSmartGaps() {
    const previous = layout.smart_gaps;
    layout.smart_gaps = !previous;
    invoke("set_layout_smart_gaps", { enabled: layout.smart_gaps }).catch(() => {
      layout.smart_gaps = previous;
    });
  }

  function toggleTiledHeaders() {
    const previous = layout.tiled_headers;
    layout.tiled_headers = !previous;
    invoke("set_layout_tiled_headers", { enabled: layout.tiled_headers }).catch(() => {
      layout.tiled_headers = previous;
    });
  }

  function persistGaps() {
    if (saveTimeout) clearTimeout(saveTimeout);
    const previous = { inner: layout.inner_gap, outer: layout.outer_gap };
    saveTimeout = setTimeout(() => {
      invoke("set_layout_gaps", { inner: layout.inner_gap, outer: layout.outer_gap }).catch(() => {
        layout.inner_gap = previous.inner;
        layout.outer_gap = previous.outer;
      });
    }, 300);
  }
</script>

<ShellPopover id="layout" width={260} right={50} bodyPadding="12px" bodyGap="10px">
  {#snippet header()}
    <PopoverHeader icon={LayoutPanelLeft} title={$t("sh.layout.title")} />
  {/snippet}

  {#if !read}
    <PopoverErrorBanner message={$t("sh.layout.stateUnknown")} />
  {:else}
  <!-- Mode Selector. The pills carry their own text labels, so no
       extra tooltips: "Single" is the monocle mode (one window at a
       time fills the workspace) in plain words. -->
  <div class="mode-section">
    <div class="mode-pills">
      <button
        class="mode-pill"
        class:active={layout.mode === "floating"}
        onclick={() => setMode("floating")}
      >
        <Layers size={16} strokeWidth={1.5} />
        <span>{$t("sh.layout.float")}</span>
      </button>
      <button
        class="mode-pill"
        class:active={layout.mode === "tiling"}
        onclick={() => setMode("tiling")}
      >
        <LayoutPanelLeft size={16} strokeWidth={1.5} />
        <span>{$t("sh.layout.tile")}</span>
      </button>
      <button
        class="mode-pill"
        class:active={layout.mode === "monocle"}
        onclick={() => setMode("monocle")}
      >
        <Maximize size={16} strokeWidth={1.5} />
        <span>{$t("sh.layout.single")}</span>
      </button>
    </div>
  </div>

  <Separator class="opacity-10" />

  <!-- Gaps -->
  <div class="gap-row">
    <span class="gap-label">{$t("sh.layout.gaps")}</span>
    <div class="gap-slider-wrap">
      <FillSlider
        value={layout.inner_gap}
        min={0}
        max={24}
        step={1}
        size="sm"
        ariaLabel={$t("sh.layout.innerGap")}
        oninput={(v) => setGap(v)}
      />
    </div>
    <span class="gap-value">{$t("sh.layout.gapValue", { px: layout.inner_gap })}</span>
  </div>

  <!-- Smart Gaps -->
  <div class="toggle-row">
    <span class="toggle-label">{$t("sh.layout.smartGaps")}</span>
    <Switch
      value={layout.smart_gaps}
      onchange={toggleSmartGaps}
      ariaLabel={$t("sh.layout.smartGaps")}
    />
  </div>

  <!--
    Title bars on tiled windows: only meaningful when tiled windows
    actually exist. Hidden in floating mode to keep the UI focused.
    The setting is global (compositor.toml [layout]) so toggling it
    in tiling/monocle and switching back to floating preserves the
    value silently.
  -->
  {#if layout.mode === "tiling" || layout.mode === "monocle"}
    <div class="toggle-row">
      <span class="toggle-label">{$t("sh.layout.titleBars")}</span>
      <Switch
        value={layout.tiled_headers}
        onchange={toggleTiledHeaders}
        ariaLabel={$t("sh.layout.tiledTitleBars")}
      />
    </div>
  {/if}
  {/if}
</ShellPopover>

<style>
  /* Mode pills */
  .mode-section { display: flex; flex-direction: column; gap: 6px; }
  .mode-pills { display: flex; gap: 4px; }
  .mode-pill {
    flex: 1; display: flex; flex-direction: column; align-items: center; gap: 4px;
    padding: 8px 4px; border-radius: var(--radius-input);
    background: transparent;
    border: 1px solid color-mix(in srgb, var(--color-fg-shell) 15%, transparent);
    color: color-mix(in srgb, var(--color-fg-shell) 50%, transparent);
    font-size: var(--text-2xs); font-weight: 500;
    transition:
      background-color var(--duration-micro, 100ms) ease,
      border-color var(--duration-micro, 100ms) ease,
      color var(--duration-micro, 100ms) ease;
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
  .gap-label { font-size: var(--text-xs); flex-shrink: 0; }
  .gap-value { font-size: var(--text-2xs); opacity: 0.5; min-width: 28px; text-align: end; }
  .gap-slider-wrap { flex: 1; display: flex; align-items: center; }

  /* Toggle row uses the same flex+gap pattern as `.gap-row` so
     the rhythm of "label, control" reads consistently across
     rows. `space-between` looks fine in isolation but creates a
     visible right-edge jitter when sibling rows have value pills
     pinned right. */
  .toggle-row { display: flex; align-items: center; gap: 10px; }
  .toggle-label {
    flex: 1;
    min-width: 0;
    font-size: var(--text-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
