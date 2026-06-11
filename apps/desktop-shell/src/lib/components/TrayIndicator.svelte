<script lang="ts">
  /// SNI system tray indicator for the top bar.
  ///
  /// Wraps the shared `Applet` primitive. Visible only when at
  /// least one StatusNotifierItem is registered. The "needs
  /// attention" state surfaces as a red dot badge so the user
  /// notices apps requesting interaction (Discord ping etc.)
  /// without having to expand the tray.

  import { togglePopover, hoverPopover, activePopover } from "$lib/stores/activePopover.js";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { Applet, AppletBadge } from "@arlen/ui-kit/components/topbar";
  import { ChevronDown } from "lucide-svelte";

  interface SniItem {
    service: string;
    id: string;
    category: string;
    status: string;
    title: string;
    icon_name: string;
    icon_pixmap: string | null;
    tooltip_title: string | null;
    tooltip_description: string | null;
    menu_path: string | null;
  }

  let items = $state<SniItem[]>([]);
  let hasAttention = $state(false);

  async function loadItems() {
    try {
      items = await invoke<SniItem[]>("get_sni_items");
      hasAttention = items.some((i) => i.status === "NeedsAttention");
    } catch {}
  }

  onMount(() => {
    loadItems();
    const unlisten = listen("sni-items-changed", () => loadItems());
    return () => {
      unlisten.then((fn) => fn());
    };
  });

  const visible = $derived(items.length > 0);
  const isOpen = $derived($activePopover === "tray");
  const tooltip = $derived(`Background Apps (${items.length})`);
</script>

{#if visible}
  <Applet
    appletId="tray"
    {tooltip}
    popoverOpen={isOpen}
    onclick={() => togglePopover("tray")}
    onmouseenter={() => hoverPopover("tray")}
  >
    {#snippet icon()}
      <ChevronDown size={14} strokeWidth={1.5} />
    {/snippet}
    {#snippet badge()}
      {#if hasAttention}
        <AppletBadge variant="dot" color="error" />
      {/if}
    {/snippet}
  </Applet>
{/if}
