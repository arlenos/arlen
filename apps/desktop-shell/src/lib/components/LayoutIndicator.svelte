<script lang="ts">
  /// Layout mode indicator for the top bar.
  ///
  /// Shows an icon reflecting the active layout mode (floating, tiling,
  /// monocle). Click opens the Layout Popover.

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { togglePopover, hoverPopover } from "$lib/stores/activePopover.js";
  import * as Tooltip from "$lib/components/ui/tooltip/index.js";
  import { Layers, LayoutPanelLeft, Maximize } from "lucide-svelte";

  let mode = $state("floating");

  const tooltipClass =
    "rounded-md border px-2 py-0.5 text-xs shadow-md select-none"
    + " bg-[var(--color-bg-shell)] text-[var(--color-fg-shell)] border-[color-mix(in_srgb,var(--color-bg-shell)_60%,white_40%)]";

  async function poll() {
    try {
      const state = await invoke<{ mode: string }>("get_layout_state");
      mode = state.mode;
    } catch {}
  }

  poll();
  onMount(() => {
    const unlisten = listen("lunaris://layout-mode-changed", (e: any) => {
      if (e.payload?.mode) mode = e.payload.mode;
    });
    return () => { unlisten.then((fn) => fn()); };
  });

  const Icon = $derived(
    mode === "tiling" ? LayoutPanelLeft :
    mode === "monocle" ? Maximize :
    Layers
  );

  const label = $derived(
    mode === "tiling" ? "Layout: Tiling" :
    mode === "monocle" ? "Layout: Monocle" :
    "Layout: Floating"
  );

  function handleClick() {
    togglePopover("layout");
  }
</script>

<Tooltip.Root>
  <Tooltip.Trigger>
    {#snippet child({ props })}
      <button
        class="layout-btn"
        aria-label={label}
        {...props}
        onclick={handleClick}
        onmouseenter={() => hoverPopover("layout")}
      >
        <Icon size={14} strokeWidth={1.5} />
      </button>
    {/snippet}
  </Tooltip.Trigger>
  <Tooltip.Portal>
    <Tooltip.Content side="bottom" class={tooltipClass}>{label}</Tooltip.Content>
  </Tooltip.Portal>
</Tooltip.Root>

<style>
  .layout-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 24px;
    min-height: 24px;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    background: transparent;
    border-radius: var(--radius-sm);
    cursor: pointer;
    color: var(--foreground);
    transition: background-color 150ms ease;
  }
  .layout-btn:hover {
    background: color-mix(in srgb, var(--foreground) 10%, transparent);
  }
</style>
