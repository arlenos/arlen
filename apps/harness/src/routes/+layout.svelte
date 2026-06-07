<script lang="ts">
  /// Root layout: the AI harness shell. Sidebar (Conversation / Agent)
  /// + a CSD titlebar, both on the `@arlen/ui-kit` canon. The window
  /// runs with `decorations: false` (Arlen CSD), so the titlebar
  /// carries the drag region and window controls.
  import "../app.css";
  import { page } from "$app/stores";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    SidebarProvider,
    SidebarInset,
    SidebarTrigger,
  } from "@arlen/ui-kit/components/ui/sidebar";
  import { Separator } from "@arlen/ui-kit/components/ui/separator";
  import HarnessSidebar from "$lib/components/HarnessSidebar.svelte";
  import WindowControls from "$lib/components/WindowControls.svelte";

  let { children } = $props();

  // The header carried only a drag region + window controls (dead centre).
  // Name the active surface there, like Settings' breadcrumb slot, so the
  // titlebar gives the user their place in the app.
  const viewTitle = $derived(
    $page.url.pathname.startsWith("/agent") ? "Agent" : "Conversation",
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
</script>

<SidebarProvider>
  <HarnessSidebar />
  <SidebarInset>
    <header
      onpointerdown={startDrag}
      ondblclick={toggleMax}
      class="flex h-12 shrink-0 items-center gap-2 border-b border-border bg-background pl-2 pr-2"
    >
      <SidebarTrigger class="-ml-1" />
      <Separator orientation="vertical" class="mr-1 h-4" />
      <span class="view-title">{viewTitle}</span>
      <div class="flex-1"></div>
      <WindowControls />
    </header>

    <main class="harness-main">
      {@render children?.()}
    </main>
  </SidebarInset>
</SidebarProvider>

<style>
  .harness-main {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .view-title {
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--foreground);
  }
</style>
