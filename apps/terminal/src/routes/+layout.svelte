<script lang="ts">
  /// Root layout: the console shell (terminal-ui-plan.md §2). Left, the
  /// terminal sidebar (sessions, history, projects); center, the block
  /// stream with the composer pinned below. The window runs with
  /// `decorations: false`; the topbar is deliberately near-empty
  /// (terminal.md §4.4: nothing essential lives in the header) — it
  /// carries only the sidebar trigger, the drag region and the window
  /// controls. cwd lives in the prompt lines, capability in the
  /// composer.
  import "../app.css";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    SidebarProvider,
    SidebarInset,
    SidebarTrigger,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { tauriAvailable } from "$lib/tauri";
  import TerminalSidebar from "$lib/components/TerminalSidebar.svelte";
  import HistoryPalette from "$lib/components/HistoryPalette.svelte";
  import { onMount } from "svelte";
  import { newSession } from "$lib/stores/sessions";
  import { historyPaletteOpen } from "$lib/stores/history";
  import { initTopbar } from "$lib/topbar";
  import { initArlenTheme } from "@arlen/ui-kit/theme";

  let { children } = $props();

  onMount(() => {
    void initTopbar();
    // Live-reskin on a desktop-wide theme switch (GAP-20).
    void initArlenTheme();
  });

  // The two global shortcuts (terminal.md §4): Ctrl+T opens a new
  // session, Ctrl+R the history palette. They work from anywhere,
  // including the composer.
  function onWindowKeydown(e: KeyboardEvent) {
    if (!e.ctrlKey || e.altKey || e.metaKey || e.shiftKey) return;
    const key = e.key.toLowerCase();
    if (key === "t") {
      e.preventDefault();
      newSession();
    } else if (key === "r") {
      e.preventDefault();
      historyPaletteOpen.update((open) => !open);
    }
  }

  // Window drag via explicit pointerdown + startDragging(), because the
  // `data-tauri-drag-region` attribute is unreliable on Wayland in
  // Tauri v2 (same approach as Settings and the harness).
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

<!-- Sidebar collapsed by default (terminal.md / Tim): the stream + composer get
     the room; the session rail opens on demand via the trigger. -->
<SidebarProvider defaultOpen={false}>
  <TerminalSidebar />
  <!-- `h-svh` locks the shell to the viewport; only the block stream
       inside the page scrolls. -->
  <SidebarInset class="h-svh">
    <header
      onpointerdown={startDrag}
      ondblclick={toggleMax}
      class="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-background pl-2 pr-2"
    >
      <SidebarTrigger class="-ml-1" />
      <div class="flex-1"></div>
      {#if tauriAvailable}
        <WindowButtons />
      {/if}
    </header>

    <div class="flex min-h-0 flex-1 flex-col">
      {@render children?.()}
    </div>
  </SidebarInset>
</SidebarProvider>

<HistoryPalette />
