<script lang="ts">
  /// Root layout: the file manager shell (file-manager-ui-plan.md).
  /// One chrome row — the headerbar carries everything: sidebar
  /// trigger, back/forward/up, the breadcrumb (outside the shell;
  /// under it the topbar shows the path), tabs, the search and info
  /// toggles, the view controls and the window buttons. Global
  /// shortcuts: Ctrl+T tab, Ctrl+W close tab, Ctrl+L edit the path,
  /// Ctrl+H hidden, Ctrl+F search, Ctrl+I info, F3 split.
  import "../app.css";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    SidebarProvider,
    SidebarInset,
    SidebarTrigger,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { IconAction } from "@arlen/ui-kit/components/ui/icon-action";
  import { Info, Search } from "lucide-svelte";
  import { tauriAvailable } from "$lib/tauri";
  import FmSidebar from "$lib/components/FmSidebar.svelte";
  import FmHeaderNav from "$lib/components/FmHeaderNav.svelte";
  import TabStrip from "$lib/components/TabStrip.svelte";
  import FmViewControls from "$lib/components/FmViewControls.svelte";
  import { newTab, closeTab, activeTabId } from "$lib/stores/tabs";
  import { focusedController, toggleSplit } from "$lib/stores/panes";
  import { homePath } from "$lib/stores/places";
  import { initTopbar, shellPresent } from "$lib/stores/topbar";
  import { initArlenTheme } from "@arlen/ui-kit/theme";
  import { onMount } from "svelte";
  import { infoOpen, pathEditing } from "$lib/stores/ui";
  import { closeSearch, searchOpen } from "$lib/stores/search";
  import { undoLast } from "$lib/stores/ops";
  import { get } from "svelte/store";

  let { children } = $props();

  onMount(() => {
    void initTopbar();
    // Live-reskin on a desktop-wide theme switch (GAP-20).
    void initArlenTheme();
  });

  function toggleSearch() {
    if (get(searchOpen)) closeSearch();
    else searchOpen.set(true);
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "F3" && !e.ctrlKey && !e.altKey && !e.metaKey) {
      e.preventDefault();
      toggleSplit();
      return;
    }
    if (!e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
    const key = e.key.toLowerCase();
    if (key === "h") {
      e.preventDefault();
      const c = get(focusedController);
      if (c) void c.setShowHidden(!get(c.showHidden));
      return;
    }
    if (key === "f") {
      e.preventDefault();
      toggleSearch();
      return;
    }
    if (key === "i") {
      e.preventDefault();
      infoOpen.update((v) => !v);
      return;
    }
    if (key === "z") {
      e.preventDefault();
      void undoLast();
    } else if (key === "t") {
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
      {#if $focusedController}
        <FmHeaderNav
          controller={$focusedController}
          homePath={$homePath}
          showCrumb={!$shellPresent}
          bind:pathEditing={$pathEditing}
        />
        {#if $shellPresent && !$pathEditing}
          <!-- No crumb in the middle: the tabs take the flexible room. -->
          <div class="flex min-w-0 flex-1 items-center gap-1">
            <TabStrip />
          </div>
        {:else}
          <TabStrip />
        {/if}
        <div class="flex items-center gap-1">
          <IconAction
            label="Search"
            size="control"
            active={$searchOpen}
            onclick={toggleSearch}
          >
            <Search size={15} strokeWidth={1.75} />
          </IconAction>
          <IconAction
            label={$infoOpen ? "Close info" : "Show info"}
            size="control"
            active={$infoOpen}
            onclick={() => infoOpen.update((v) => !v)}
          >
            <Info size={15} strokeWidth={1.75} />
          </IconAction>
        </div>
        <FmViewControls />
      {:else}
        <div class="flex-1"></div>
      {/if}
      {#if tauriAvailable}
        <WindowButtons />
      {/if}
    </header>

    <div class="flex min-h-0 flex-1 flex-col">
      {@render children?.()}
    </div>
  </SidebarInset>
</SidebarProvider>
