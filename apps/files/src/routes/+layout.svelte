<script lang="ts">
  /// Root layout: the file manager shell (file-manager-ui-plan.md).
  /// Headerless CSD like the terminal: a slim drag strip with the
  /// sidebar trigger and the window controls; everything essential
  /// lives in the toolbar and the content below. Global shortcuts:
  /// Ctrl+T tab, Ctrl+W close tab, Ctrl+L edit the path.
  import "../app.css";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    SidebarProvider,
    SidebarInset,
    SidebarTrigger,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { tauriAvailable } from "$lib/tauri";
  import FmSidebar from "$lib/components/FmSidebar.svelte";
  import TabStrip from "$lib/components/TabStrip.svelte";
  import FmViewControls from "$lib/components/FmViewControls.svelte";
  import { newTab, closeTab, activeTabId } from "$lib/stores/tabs";
  import { pathEditing } from "$lib/stores/ui";
  import { get } from "svelte/store";

  let { children } = $props();

  function onWindowKeydown(e: KeyboardEvent) {
    if (!e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
    const key = e.key.toLowerCase();
    if (key === "t") {
      e.preventDefault();
      newTab();
    } else if (key === "w") {
      e.preventDefault();
      const id = get(activeTabId);
      if (id !== null) closeTab(id);
    } else if (key === "l") {
      e.preventDefault();
      pathEditing.set(true);
    }
  }

  // Window drag via explicit pointerdown + startDragging(), because the
  // `data-tauri-drag-region` attribute is unreliable on Wayland in
  // Tauri v2 (same approach as Settings, the harness and the terminal).
  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }
  async function startDrag(e: PointerEvent) {
    if (!tauriAvailable) return;
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    await getCurrentWindow().startDragging();
  }
  async function toggleMax(e: MouseEvent) {
    if (!tauriAvailable) return;
    if (isInteractive(e)) return;
    const w = getCurrentWindow();
    (await w.isMaximized()) ? await w.unmaximize() : await w.maximize();
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<SidebarProvider>
  <FmSidebar />
  <SidebarInset class="h-svh">
    <header
      onpointerdown={startDrag}
      ondblclick={toggleMax}
      class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-background pl-2 pr-2"
    >
      <SidebarTrigger class="-ml-1" />
      <div class="flex min-w-0 flex-1 items-center gap-1 pl-1">
        <TabStrip />
      </div>
      <FmViewControls />
      {#if tauriAvailable}
        <WindowButtons />
      {/if}
    </header>

    <div class="flex min-h-0 flex-1 flex-col">
      {@render children?.()}
    </div>
  </SidebarInset>
</SidebarProvider>
