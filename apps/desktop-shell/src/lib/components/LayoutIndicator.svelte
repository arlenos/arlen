<script lang="ts">
  import { t } from "$lib/i18n/messages";
  import { shellRead } from "$lib/shellRead";
  /// Layout-mode indicator for the top bar.
  ///
  /// Wraps the shared `Applet` primitive. Reflects the current
  /// compositor layout mode (floating / tiling / monocle) via icon
  /// swap. Click toggles the LayoutPopover.

  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { togglePopover, hoverPopover, activePopover } from "$lib/stores/activePopover.js";
  import { Applet } from "@arlen/ui-kit/components/topbar";
  import { Layers, LayoutPanelLeft, Maximize } from "lucide-svelte";

  let mode = $state("floating");

  async function poll() {
    // Keeps the last known mode rather than inventing one, and says in the log
    // when it could not be read: a poll that has been failing for an hour looks
    // identical to a layout that has not changed.
    const s = await shellRead<{ mode: string }>("get_layout_state", "layout");
    if (s !== null) mode = s.mode;
  }

  poll();

  onMount(() => {
    const unlisten = listen("arlen://layout-mode-changed", (e: any) => {
      if (e.payload?.mode) mode = e.payload.mode;
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  });

  const Icon = $derived(
    mode === "tiling" ? LayoutPanelLeft : mode === "monocle" ? Maximize : Layers,
  );
  // The mode name is the same key the layout panel's pills use, so the bar and
  // the panel it opens cannot end up calling one mode two things. These were
  // three English literals until 6 September, on the strip that is always up.
  const tooltip = $derived(
    $t("sh.layout.tip", {
      mode:
        mode === "tiling"
          ? $t("sh.layout.tile")
          : mode === "monocle"
            ? $t("sh.layout.single")
            : $t("sh.layout.float"),
    }),
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
