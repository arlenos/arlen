<script lang="ts">
  /// App shell: a slim CSD titlebar over the meeting view. The window runs with
  /// `decorations: false`, so without this the toplevel is a naked frameless window
  /// (no drag, no min/max/close). Drag is an explicit `startDragging()` pointerdown
  /// (the `data-tauri-drag-region` attribute is unreliable on Wayland in Tauri v2),
  /// mirroring `apps/settings` SiteHeader; the buttons are the shared ui-kit cluster.
  import "../app.css";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { WindowButtons } from "@arlen/ui-kit/components/ui/window-controls";
  import { t } from "$lib/i18n/messages";

  let { children } = $props();

  function isInteractive(e: Event): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("button, a, input, [role='button']");
  }

  async function startDrag(e: PointerEvent) {
    if (e.button !== 0 || e.pointerType !== "mouse") return;
    if (isInteractive(e)) return;
    try {
      await getCurrentWindow().startDragging();
    } catch {
      /* standalone (vite) has no toplevel to drag */
    }
  }

  async function toggleMax(e: MouseEvent) {
    if (isInteractive(e)) return;
    try {
      const w = getCurrentWindow();
      (await w.isMaximized()) ? await w.unmaximize() : await w.maximize();
    } catch {
      /* no window in standalone */
    }
  }
</script>

<div class="flex h-screen flex-col">
  <!-- The header is a drag surface (a non-keyboard pointer interaction); its actual
       controls are the accessible WindowButtons, so the static-interaction lint is a
       false positive here. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header
    onpointerdown={startDrag}
    ondblclick={toggleMax}
    class="flex h-10 shrink-0 items-center justify-between border-b border-border bg-background ps-3 pe-1"
  >
    <span class="select-none text-sm font-medium text-foreground">{$t("mt.title")}</span>
    <WindowButtons />
  </header>
  <main class="min-h-0 flex-1 overflow-auto">
    {@render children()}
  </main>
</div>
