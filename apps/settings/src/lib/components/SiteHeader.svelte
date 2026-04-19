<script lang="ts">
  /// Top bar: sidebar trigger + breadcrumbs + window controls.
  ///
  /// Window drag is wired via an explicit pointerdown handler calling
  /// `startDragging()` rather than `data-tauri-drag-region`, because
  /// the attribute-based path is unreliable on Wayland in Tauri v2.
  import { breadcrumbs } from "$lib/stores/navigation";
  import { SidebarTrigger } from "$lib/components/ui/sidebar";
  import { Separator } from "$lib/components/ui/separator";
  import WindowControls from "./WindowControls.svelte";
  import { ChevronRight } from "lucide-svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

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

<!-- Right-click on the titlebar is currently a no-op. The Lunaris
     compositor owns the canonical window menu for native Wayland
     toplevels; ultimately the titlebar plugin should request it via a
     new `show_window_menu` message on `lunaris-titlebar-v1`, so CSD
     apps get the exact same menu (Move to Workspace submenu included)
     rather than a divergent copy. Until that lands, no menu on
     right-click is better than two different ones. -->
<header
  onpointerdown={startDrag}
  ondblclick={toggleMax}
  class="flex h-12 shrink-0 items-center gap-2 border-b border-border bg-background pl-2 pr-2"
>
  <div class="flex items-center gap-2">
    <SidebarTrigger class="-ml-1" />
    <Separator orientation="vertical" class="mr-1 h-4" />
    <nav
      class="flex items-center gap-1.5 text-sm text-muted-foreground"
      aria-label="Breadcrumb"
    >
      {#each $breadcrumbs as segment, i (segment + i)}
        {#if i > 0}
          <ChevronRight size={14} class="opacity-50" />
        {/if}
        <span
          class={i === $breadcrumbs.length - 1
            ? "font-medium text-foreground"
            : ""}
        >
          {segment}
        </span>
      {/each}
    </nav>
  </div>

  <div class="flex-1"></div>

  <WindowControls />
</header>
