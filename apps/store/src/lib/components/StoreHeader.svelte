<script lang="ts">
  /// The header bar on the files canon: h-10, trigger first, the PLACE in the
  /// middle (the app name lives in the sidebar caps label). Window calls are
  /// guarded so the app renders under vite.
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { SidebarTrigger } from "@arlen/ui-kit/components/ui/sidebar";
  import { Separator } from "@arlen/ui-kit/components/ui/separator";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  let { placeLabel }: { placeLabel: string } = $props();

  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }

  async function startDrag(e: PointerEvent): Promise<void> {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      // No Tauri runtime under vite.
    }
  }

  async function toggleMax(e: MouseEvent): Promise<void> {
    if (isInteractive(e)) return;
    try {
      const w = getCurrentWindow();
      (await w.isMaximized()) ? await w.unmaximize() : await w.maximize();
    } catch {
      // vite: no-op.
    }
  }
</script>

<!-- The header is a drag surface (a non-keyboard pointer interaction); its actual
     controls are the accessible WindowButtons inside it, so the static-interaction
     lint is a false positive here. Same treatment as the meetings layout. -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<header
  class="flex h-10 shrink-0 select-none items-center gap-2 border-b border-border bg-background px-2"
  onpointerdown={startDrag}
  ondblclick={toggleMax}
>
  <SidebarTrigger class="-ml-1" />
  <Separator orientation="vertical" class="me-1 h-4" />
  <span class="truncate text-sm font-medium text-foreground">{placeLabel}</span>
  <span class="flex-1"></span>
  <WindowButtons />
</header>
