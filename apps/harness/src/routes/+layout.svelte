<script lang="ts">
  /// Root layout: the harness three-pane shell (ai-app.md §2.0). Left, the
  /// sidebar (surface nav + conversation history); center, the active
  /// surface; a surface brings its own contextual right pane. The window
  /// runs with `decorations: false` (Arlen CSD), so the slim header carries
  /// only the drag region, the surface title, and the window controls —
  /// nothing essential lives in it (terminal.md §4.4).
  import "../app.css";
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    SidebarProvider,
    SidebarInset,
    SidebarTrigger,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Separator } from "@arlen/ui-kit/components/ui/separator";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import HarnessSidebar from "$lib/components/HarnessSidebar.svelte";
  import TransparencyDrawer from "$lib/components/chat/TransparencyDrawer.svelte";
  import { activeTitle, initSessions, newSession } from "$lib/stores/conversation";

  let { children } = $props();

  // Load persisted conversations here, not in a route, so the history in the
  // sidebar is populated whichever surface the app opens on.
  onMount(() => {
    initSessions();
  });

  // The header names the place. On the conversation route it shows the active
  // conversation's title, so the user has their bearings even when the sidebar
  // is collapsed; the agent route and an empty conversation fall back to the
  // surface name.
  const viewTitle = $derived(
    $page.url.pathname.startsWith("/agent") ? "Activity" : $activeTitle || "Chat",
  );

  // Window drag via explicit pointerdown + startDragging(), because the
  // `data-tauri-drag-region` attribute is unreliable on Wayland in
  // Tauri v2 (same approach as the Settings app).
  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }
  async function startDrag(e: PointerEvent) {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    await getCurrentWindow().startDragging();
  }
  async function toggleMax(e: MouseEvent) {
    if (isInteractive(e)) return;
    const w = getCurrentWindow();
    (await w.isMaximized()) ? await w.unmaximize() : await w.maximize();
  }

  // Ctrl+N starts a new chat, Ctrl+K jumps to the history search. The
  // affordances live in the sidebar; the shortcuts are global so they work
  // with the sidebar collapsed.
  function onKeydown(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey) return;
    const key = e.key.toLowerCase();
    if (key === "n") {
      e.preventDefault();
      newSession();
      if ($page.url.pathname !== "/") goto("/");
    } else if (key === "k") {
      e.preventDefault();
      document.getElementById("harness-session-search")?.focus();
    }
  }
</script>

<svelte:window onkeydown={onKeydown} />

<SidebarProvider>
  <HarnessSidebar />
  <!-- `h-svh` locks the shell to the viewport so the CSD header never
       scrolls away; only the content region below it scrolls. -->
  <SidebarInset class="h-svh">
    <header
      onpointerdown={startDrag}
      ondblclick={toggleMax}
      class="flex h-12 shrink-0 items-center gap-2 border-b border-border bg-background pl-2 pr-2"
    >
      <SidebarTrigger class="-ml-1" />
      <Separator orientation="vertical" class="mr-1 h-4" />
      <span class="text-sm font-medium text-foreground">{viewTitle}</span>
      <div class="flex-1"></div>
      <WindowButtons />
    </header>

    <!-- SidebarInset is the page's <main>; this is just its scroll region. -->
    <div class="min-h-0 flex-1 overflow-y-auto">
      {@render children?.()}
    </div>
  </SidebarInset>
</SidebarProvider>

<!-- The transparency drawer overlays the whole shell, summoned from the
     composer foot; mounted once here so it works from any surface. -->
<TransparencyDrawer />
