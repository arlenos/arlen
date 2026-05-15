<script lang="ts">
  /// Layout-mode indicator for the top bar.
  ///
  /// Wraps the shared `Applet` primitive. Reflects the current
  /// compositor layout mode (floating / tiling / monocle) via icon
  /// swap. Click toggles the LayoutPopover.

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { togglePopover, hoverPopover, activePopover } from "$lib/stores/activePopover.js";
  import { Applet } from "@lunaris/ui-kit/components/topbar";
  import { Layers, LayoutPanelLeft, Maximize } from "lucide-svelte";

  let mode = $state("floating");

  async function poll() {
    try {
      const s = await invoke<{ mode: string }>("get_layout_state");
      mode = s.mode;
    } catch {}
  }

  poll();

  onMount(() => {
    const unlisten = listen("lunaris://layout-mode-changed", (e: any) => {
      if (e.payload?.mode) mode = e.payload.mode;
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  });

  const Icon = $derived(
    mode === "tiling" ? LayoutPanelLeft : mode === "monocle" ? Maximize : Layers,
  );
  const tooltip = $derived(
    mode === "tiling"
      ? "Layout: Tiling"
      : mode === "monocle"
        ? "Layout: Monocle"
        : "Layout: Floating",
  );

  const isOpen = $derived($activePopover === "layout");
</script>

<Applet
  appletId="layout"
  {tooltip}
  popoverOpen={isOpen}
  onclick={() => togglePopover("layout")}
  onmouseenter={() => hoverPopover("layout")}
>
  {#snippet icon()}
    <Icon size={14} strokeWidth={1.5} />
  {/snippet}
</Applet>
